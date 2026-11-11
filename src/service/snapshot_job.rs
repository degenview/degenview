//! What happens to a snapshot once it is drawn: picture it, then save it, hand it to the clipboard
//! or upload it. Everything that waits (the save panel, the rasterizing, the PNG, the network)
//! runs here, on a worker; what has to happen on the UI thread (the clipboard, the browser) is
//! handed back as an [`Outcome`] for the window to carry out, so the app never reports a copy or
//! an open that did not happen.

use crate::model::chart::snapshot::{SharedSnapshot, SnapshotAction, UploadError};
use crate::service::keychain_snapshot_secrets::SnapshotSecrets;
use crate::service::snapshot_export::{self, ExportError};
use crate::service::snapshot_providers::{Destination, Purpose};
use crate::service::snapshot_raster::{self, Scene};
use crate::service::snapshot_upload::{self, SnapshotTransport, UreqTransport};
use eframe::egui::ColorImage;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The parts of the outside world a job touches, so tests can stand in for them.
pub trait Backend: Send + Sync {
    /// Where to save a file called `name`; `None` when the user cancels.
    fn choose_destination(&self, name: &str) -> Result<Option<PathBuf>, ExportError>;
    fn write(&self, path: &Path, png: &[u8]) -> Result<(), ExportError>;
    fn upload(&self, png: &[u8]) -> Result<SharedSnapshot, UploadError>;
    /// Where uploads go, as the user reads it, for what is said when one fails.
    fn label(&self) -> String {
        "the snapshot host".to_owned()
    }
}

/// The real thing: the system's save panel, the file system and the provider the user turned on.
pub struct SystemBackend {
    pub destination: Option<Destination>,
    /// Read on the worker, when an upload needs them: a secure-storage prompt must never stop
    /// the window.
    pub secrets: Arc<SnapshotSecrets>,
    pub transport: Arc<dyn SnapshotTransport>,
}

impl SystemBackend {
    pub fn new(destination: Option<Destination>) -> Self {
        Self {
            destination,
            secrets: SnapshotSecrets::shared(),
            transport: Arc::new(UreqTransport::default()),
        }
    }
}

impl Backend for SystemBackend {
    fn choose_destination(&self, name: &str) -> Result<Option<PathBuf>, ExportError> {
        snapshot_export::choose_destination(name)
    }

    fn write(&self, path: &Path, png: &[u8]) -> Result<(), ExportError> {
        snapshot_export::write_atomic(path, png)
    }

    fn upload(&self, png: &[u8]) -> Result<SharedSnapshot, UploadError> {
        let Some(destination) = &self.destination else {
            return Err(UploadError::Offline);
        };
        let secrets = self.secrets.read_all(destination.provider());
        snapshot_upload::upload(&*self.transport, destination, &secrets, png, Purpose::Share)
    }

    fn label(&self) -> String {
        self.destination
            .as_ref()
            .map_or_else(|| "the snapshot host".to_owned(), Destination::label)
    }
}

/// An upload that failed in a way that may pass: the image is kept so trying again costs
/// nothing but the network.
#[derive(Clone, Debug)]
pub struct Retry {
    pub action: SnapshotAction,
    pub png: Arc<Vec<u8>>,
}

#[derive(Debug)]
pub enum Outcome {
    /// The file is written.
    Saved(PathBuf),
    /// The user dismissed the save panel: nothing to say.
    Cancelled,
    /// The picture, for the window to put on the clipboard.
    CopyImage(ColorImage),
    /// The host stored the image: for the window to copy, open or share.
    Hosted {
        action: SnapshotAction,
        shared: SharedSnapshot,
    },
    Failed {
        message: String,
        retry: Option<Retry>,
    },
}

fn failed(message: impl Into<String>) -> Outcome {
    Outcome::Failed {
        message: message.into(),
        retry: None,
    }
}

/// Draws `scene` and encodes it.
fn picture(scene: &Scene) -> Result<ColorImage, Outcome> {
    snapshot_raster::rasterize(scene).map_err(|_| failed("Could not draw the image."))
}

fn png_of(image: &ColorImage) -> Result<Vec<u8>, Outcome> {
    snapshot_export::encode_png(image).map_err(|error| failed(error.message()))
}

/// Uploads `png` for `action`; a failure that may pass keeps the image to try again with.
fn host(action: SnapshotAction, png: Arc<Vec<u8>>, backend: &dyn Backend) -> Outcome {
    match backend.upload(&png) {
        Ok(shared) => Outcome::Hosted { action, shared },
        Err(error) => Outcome::Failed {
            message: error.message(&backend.label()),
            retry: error.retryable().then_some(Retry { action, png }),
        },
    }
}

/// Carries `action` out for the picture `scene` holds. `name` is the file name offered when saving.
pub fn run(action: SnapshotAction, scene: &Scene, name: &str, backend: &dyn Backend) -> Outcome {
    match action {
        SnapshotAction::Download => {
            // Ask first: the panel is what the user is waiting for, and a cancel costs nothing.
            let path = match backend.choose_destination(name) {
                Ok(Some(path)) => path,
                Ok(None) => return Outcome::Cancelled,
                Err(error) => return failed(error.message()),
            };
            let png = match picture(scene).and_then(|image| png_of(&image)) {
                Ok(png) => png,
                Err(outcome) => return outcome,
            };
            match backend.write(&path, &png) {
                Ok(()) => Outcome::Saved(path),
                Err(error) => failed(error.message()),
            }
        }
        SnapshotAction::CopyImage => match picture(scene) {
            Ok(image) => Outcome::CopyImage(image),
            Err(outcome) => outcome,
        },
        SnapshotAction::CopyLink | SnapshotAction::OpenInTab | SnapshotAction::ShareOnX => {
            match picture(scene).and_then(|image| png_of(&image)) {
                Ok(png) => host(action, Arc::new(png), backend),
                Err(outcome) => outcome,
            }
        }
    }
}

/// Uploads a kept image again.
pub fn retry(retry: Retry, backend: &dyn Backend) -> Outcome {
    host(retry.action, retry.png, backend)
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Color32, TexturesDelta};
    use std::sync::Mutex;

    /// A 4 × 3 scene of nothing but its background.
    fn scene() -> Scene {
        Scene::new(
            Vec::new(),
            TexturesDelta::default(),
            1.0,
            (4, 3),
            Color32::from_rgb(1, 2, 3),
        )
    }

    #[derive(Default)]
    struct Fake {
        destination: Mutex<Option<Result<Option<PathBuf>, ExportError>>>,
        written: Mutex<Vec<(PathBuf, Vec<u8>)>>,
        write_error: Mutex<Option<ExportError>>,
        uploads: Mutex<Vec<Vec<u8>>>,
        answers: Mutex<Vec<Result<SharedSnapshot, UploadError>>>,
        asked_for: Mutex<Vec<String>>,
    }

    impl Backend for Fake {
        fn choose_destination(&self, name: &str) -> Result<Option<PathBuf>, ExportError> {
            self.asked_for.lock().unwrap().push(name.to_owned());
            self.destination
                .lock()
                .unwrap()
                .take()
                .expect("a destination was set up")
        }

        fn write(&self, path: &Path, png: &[u8]) -> Result<(), ExportError> {
            if let Some(error) = self.write_error.lock().unwrap().take() {
                return Err(error);
            }
            self.written
                .lock()
                .unwrap()
                .push((path.to_owned(), png.to_vec()));
            Ok(())
        }

        fn upload(&self, png: &[u8]) -> Result<SharedSnapshot, UploadError> {
            self.uploads.lock().unwrap().push(png.to_vec());
            self.answers.lock().unwrap().remove(0)
        }
    }

    fn shared() -> SharedSnapshot {
        SharedSnapshot {
            url: "https://s.example/a.png".into(),
            delete_url: None,
            expires_at: None,
        }
    }

    #[test]
    fn download_asks_where_then_writes_a_png_of_the_scene() {
        let fake = Fake::default();
        *fake.destination.lock().unwrap() = Ok(Some(PathBuf::from("/x/chart.png"))).into();
        let outcome = run(
            SnapshotAction::Download,
            &scene(),
            "DegenView_BTC.png",
            &fake,
        );
        assert!(matches!(&outcome, Outcome::Saved(path) if path == Path::new("/x/chart.png")));
        assert_eq!(*fake.asked_for.lock().unwrap(), ["DegenView_BTC.png"]);
        let written = fake.written.lock().unwrap();
        let decoded = image::load_from_memory(&written[0].1)
            .expect("a png")
            .to_rgba8();
        assert_eq!(decoded.dimensions(), (4, 3));
        assert_eq!(decoded.get_pixel(0, 0).0, [1, 2, 3, 255]);
    }

    #[test]
    fn cancelling_the_save_panel_is_silent_and_writes_nothing() {
        let fake = Fake::default();
        *fake.destination.lock().unwrap() = Ok(None).into();
        assert!(matches!(
            run(SnapshotAction::Download, &scene(), "a.png", &fake),
            Outcome::Cancelled
        ));
        assert!(fake.written.lock().unwrap().is_empty());
    }

    #[test]
    fn a_save_that_fails_says_why_and_never_claims_success() {
        let fake = Fake::default();
        *fake.destination.lock().unwrap() = Ok(Some(PathBuf::from("/x/chart.png"))).into();
        *fake.write_error.lock().unwrap() = Some(ExportError::Write("the disk is full".into()));
        match run(SnapshotAction::Download, &scene(), "a.png", &fake) {
            Outcome::Failed { message, retry } => {
                assert!(message.contains("the disk is full"), "{message}");
                assert!(retry.is_none());
            }
            other => panic!("{other:?}"),
        }
        let broken = Fake::default();
        *broken.destination.lock().unwrap() = Some(Err(ExportError::Dialog("no dialog".into())));
        assert!(matches!(
            run(SnapshotAction::Download, &scene(), "a.png", &broken),
            Outcome::Failed { .. }
        ));
    }

    #[test]
    fn copying_the_image_hands_back_its_pixels_and_uploads_nothing() {
        let fake = Fake::default();
        match run(SnapshotAction::CopyImage, &scene(), "a.png", &fake) {
            Outcome::CopyImage(image) => {
                assert_eq!(image.size, [4, 3]);
                assert!(
                    image
                        .pixels
                        .iter()
                        .all(|p| *p == Color32::from_rgb(1, 2, 3))
                );
            }
            other => panic!("{other:?}"),
        }
        assert!(fake.uploads.lock().unwrap().is_empty());
        assert!(fake.asked_for.lock().unwrap().is_empty());
    }

    #[test]
    fn the_hosted_actions_upload_the_png_and_return_the_link() {
        for action in [
            SnapshotAction::CopyLink,
            SnapshotAction::OpenInTab,
            SnapshotAction::ShareOnX,
        ] {
            let fake = Fake::default();
            fake.answers.lock().unwrap().push(Ok(shared()));
            match run(action, &scene(), "a.png", &fake) {
                Outcome::Hosted {
                    action: done,
                    shared,
                } => {
                    assert_eq!(done, action);
                    assert_eq!(shared.url, "https://s.example/a.png");
                }
                other => panic!("{other:?}"),
            }
            let uploads = fake.uploads.lock().unwrap();
            assert_eq!(uploads.len(), 1);
            assert_eq!(&uploads[0][..8], b"\x89PNG\r\n\x1a\n");
            assert!(fake.written.lock().unwrap().is_empty(), "nothing is saved");
        }
    }

    #[test]
    fn a_failure_that_may_pass_keeps_the_image_for_a_retry() {
        let fake = Fake::default();
        fake.answers.lock().unwrap().push(Err(UploadError::Offline));
        let Outcome::Failed {
            message,
            retry: Some(kept),
        } = run(SnapshotAction::CopyLink, &scene(), "a.png", &fake)
        else {
            panic!("a retry was due");
        };
        assert!(message.contains("Could not reach"), "{message}");
        // The retry sends the very same bytes, without drawing the chart again.
        fake.answers.lock().unwrap().push(Ok(shared()));
        let again = self::retry(kept, &fake);
        assert!(matches!(
            again,
            Outcome::Hosted {
                action: SnapshotAction::CopyLink,
                ..
            }
        ));
        let uploads = fake.uploads.lock().unwrap();
        assert_eq!(uploads.len(), 2);
        assert_eq!(uploads[0], uploads[1]);
    }

    #[test]
    fn a_failure_that_will_not_pass_is_not_offered_again() {
        for error in [
            UploadError::Unauthorized,
            UploadError::RejectedAsTooLarge,
            UploadError::BadResponse,
        ] {
            let fake = Fake::default();
            fake.answers.lock().unwrap().push(Err(error));
            assert!(matches!(
                run(SnapshotAction::OpenInTab, &scene(), "a.png", &fake),
                Outcome::Failed { retry: None, .. }
            ));
        }
    }

    #[test]
    fn a_scene_that_cannot_be_drawn_is_a_failure_not_a_panic() {
        let empty = Scene::new(
            Vec::new(),
            TexturesDelta::default(),
            1.0,
            (0, 0),
            Color32::BLACK,
        );
        let fake = Fake::default();
        assert!(matches!(
            run(SnapshotAction::CopyImage, &empty, "a.png", &fake),
            Outcome::Failed { .. }
        ));
    }
}
