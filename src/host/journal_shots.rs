//! Screenshots for the Trade Journal: reading a picture on a worker, keeping it in the asset store
//! (the one drawings use, content-addressed), and holding the textures the thumbnails show.
//!
//! Nothing here uploads anything. A picture is checked and shrunk by `image_ingest` (PNG or JPEG,
//! at most 600 kB stored), stored once by the hash of its bytes, and named by that hash from then
//! on. Removing a screenshot from a review leaves the picture in the store, as for drawings: a
//! drawing, another review or another Mac may still show it.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};

use eframe::egui::{self, ColorImage};

use crate::service::asset_host::{self, AssetImage};
use crate::service::{file_dialog, image_ingest, work_queue};
use crate::store::SharedStore;
use crate::store::asset_store::StoredAsset;
use crate::ui::journal_shots::Thumb;

pub enum Source {
    /// The system file chooser.
    Panel,
    Clipboard,
    Path(PathBuf),
}

enum Outcome {
    Stored { hash: String, pixels: ColorImage },
    Cancelled,
    Failed(String),
}

#[derive(Default)]
pub struct Shots {
    textures: HashMap<String, (egui::TextureHandle, [usize; 2])>,
    inbox: Option<Receiver<Outcome>>,
    pub error: Option<String>,
    /// The picture shown full size.
    pub viewing: Option<String>,
}

impl Shots {
    pub fn busy(&self) -> bool {
        self.inbox.is_some()
    }

    /// Starts reading a picture. One at a time.
    pub fn start(&mut self, ctx: &egui::Context, store: SharedStore, source: Source) {
        if self.busy() {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        self.inbox = Some(receiver);
        self.error = None;
        let ctx = ctx.clone();
        work_queue::ASSETS.submit(move || {
            let _ = sender.send(work(source, &store));
            ctx.request_repaint();
        });
    }

    /// The hash of a picture that finished storing since the last call.
    pub fn poll(&mut self, ctx: &egui::Context) -> Option<String> {
        let outcome = match self.inbox.as_ref()?.try_recv() {
            Ok(outcome) => outcome,
            Err(mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(120));
                return None;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                Outcome::Failed("The picture could not be read.".into())
            }
        };
        self.inbox = None;
        match outcome {
            Outcome::Stored { hash, pixels } => {
                if let Some(host) = asset_host::host(ctx) {
                    host.insert_ready(&hash, pixels);
                }
                Some(hash)
            }
            Outcome::Cancelled => None,
            Outcome::Failed(message) => {
                self.error = Some(message);
                None
            }
        }
    }

    /// What `hash` is now, loading its texture the first time its pixels are ready.
    pub fn thumb(&mut self, ctx: &egui::Context, hash: &str) -> Thumb {
        if let Some((texture, size)) = self.textures.get(hash) {
            return Thumb::Ready {
                texture: texture.id(),
                size: *size,
            };
        }
        match asset_host::lookup(ctx, hash) {
            AssetImage::Ready(pixels) => {
                let size = pixels.size;
                let texture = ctx.load_texture(
                    format!("journal-shot-{hash}"),
                    (*pixels).clone(),
                    egui::TextureOptions::LINEAR,
                );
                let id = texture.id();
                self.textures.insert(hash.to_owned(), (texture, size));
                Thumb::Ready { texture: id, size }
            }
            AssetImage::Loading => Thumb::Loading,
            AssetImage::Missing => Thumb::Missing,
            AssetImage::Corrupt => Thumb::Corrupt,
        }
    }

    /// Lets go of the textures of pictures no longer on screen.
    pub fn keep_only(&mut self, hashes: &[String]) {
        self.textures.retain(|hash, _| hashes.contains(hash));
    }
}

fn work(source: Source, store: &SharedStore) -> Outcome {
    let bytes = match source {
        Source::Panel => match file_dialog::choose_image("Choose a screenshot") {
            Ok(Some(path)) => image_ingest::read_file(&path),
            Ok(None) => return Outcome::Cancelled,
            Err(message) => return Outcome::Failed(message),
        },
        Source::Path(path) => image_ingest::read_file(&path),
        Source::Clipboard => match crate::native_bridge::clipboard_picture() {
            Some(bytes) => Ok(bytes),
            None => return Outcome::Failed("There is no picture on the clipboard.".into()),
        },
    };
    store_bytes(bytes, store)
}

fn store_bytes(bytes: Result<Vec<u8>, image_ingest::IngestError>, store: &SharedStore) -> Outcome {
    let picture = match bytes.and_then(|bytes| image_ingest::ingest(&bytes)) {
        Ok(picture) => picture,
        Err(error) => return Outcome::Failed(error.message().to_owned()),
    };
    let Some(pixels) = image_ingest::pixels(&picture.bytes) else {
        return Outcome::Failed(image_ingest::IngestError::Corrupt.message().to_owned());
    };
    let stored = store
        .lock()
        .map_err(|_| "The library is busy.".to_owned())
        .and_then(|store| {
            store
                .put_asset(&StoredAsset::from_ingested(&picture))
                .map_err(|error| format!("The picture could not be saved: {error}"))
        });
    match stored {
        Ok(_) => Outcome::Stored {
            hash: picture.id,
            pixels,
        },
        Err(message) => Outcome::Failed(message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;
    use std::sync::{Arc, Mutex};

    pub(crate) fn png(seed: u8) -> Vec<u8> {
        let image =
            image::RgbaImage::from_fn(40, 24, |x, y| image::Rgba([seed, x as u8, y as u8, 255]));
        let mut png = Vec::new();
        image::ImageEncoder::write_image(
            image::codecs::png::PngEncoder::new(&mut png),
            image.as_raw(),
            40,
            24,
            image::ExtendedColorType::Rgba8,
        )
        .unwrap();
        png
    }

    fn shared() -> SharedStore {
        Arc::new(Mutex::new(Store::open_in_memory().unwrap()))
    }

    #[test]
    fn a_picture_is_stored_once_under_its_hash() {
        let store = shared();
        let Outcome::Stored { hash, pixels } = store_bytes(Ok(png(7)), &store) else {
            panic!("stored");
        };
        assert_eq!(pixels.size, [40, 24]);
        assert!(crate::model::journal::reviews::is_picture_hash(&hash));
        let again = store_bytes(Ok(png(7)), &store);
        assert!(matches!(&again, Outcome::Stored { hash: other, .. } if *other == hash));
        let kept = store.lock().unwrap().asset(&hash).unwrap().unwrap();
        assert_eq!(kept.width, 40);
        assert!(kept.verified_bytes().is_some());
    }

    #[test]
    fn something_that_is_not_a_picture_is_refused_and_nothing_is_stored() {
        let store = shared();
        let Outcome::Failed(message) = store_bytes(Ok(b"hello".to_vec()), &store) else {
            panic!("refused");
        };
        assert!(!message.is_empty());
        let Outcome::Failed(_) = store_bytes(Err(image_ingest::IngestError::TooLarge), &store)
        else {
            panic!("too large");
        };
    }

    #[test]
    fn a_stored_picture_gets_a_texture_once_the_host_has_decoded_it() {
        let store = shared();
        let ctx = egui::Context::default();
        let host = asset_host::AssetHost::new(store.clone());
        asset_host::install(&ctx, host.clone());
        let Outcome::Stored { hash, .. } = store_bytes(Ok(png(9)), &store) else {
            panic!("stored");
        };
        let mut shots = Shots::default();
        // Not asked for yet: the host starts loading it.
        assert!(!matches!(shots.thumb(&ctx, &hash), Thumb::Ready { .. }));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if let Thumb::Ready { size, .. } = shots.thumb(&ctx, &hash) {
                assert_eq!(size, [40, 24]);
                break;
            }
            assert!(std::time::Instant::now() < deadline, "never decoded");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        shots.keep_only(&[]);
        assert!(shots.textures.is_empty());
    }
}
