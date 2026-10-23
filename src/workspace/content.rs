//! Choosing what a content tool places: where a picture comes from, how it is read, and what the
//! chart is told.
//!
//! Arming the Image tool opens a sheet; a picture is chosen with the file panel, from the
//! clipboard, or by dropping it on the window. Reading, checking, scaling, hashing and storing
//! all happen on a worker (never on the UI thread), and the result comes back as one message: the
//! chosen picture (a name for the store, a size) waiting to be placed by a click, or placed at
//! once in the middle of the focused chart when it came by a drop or a paste, or swapped in for
//! the picture of a drawing being replaced. Nothing is created until the picture is stored, so a
//! refused picture leaves no drawing, no undo step and nothing to sync.

use super::ChartWorkspace;
use crate::model::Tool;
use crate::model::image_content::{Content, ImageContent, MAX_NAME_CHARS};
use crate::service::asset_host;
use crate::service::image_ingest::{self, IngestError};
use crate::service::{file_dialog, work_queue};
use crate::store::SharedStore;
use crate::store::asset_store::StoredAsset;
use crate::ui::content_sheet::{self, ImageSheet, ImageSheetAction};
use eframe::egui::{self, ColorImage};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};

/// Where a picture comes from.
pub(super) enum Source {
    /// The system's file panel.
    Panel,
    Clipboard,
    /// A file the user dropped, by its path.
    Path(PathBuf),
}

/// What the picture that is being read is for.
#[derive(Clone, Copy)]
pub(super) enum Target {
    /// To be placed: by the next click, or at once (a drop or a paste).
    Place { at_once: bool },
    /// To replace the picture of this drawing.
    Replace { chart_id: u64, drawing_id: u64 },
}

/// What the worker found.
enum Outcome {
    Stored {
        content: ImageContent,
        pixels: ColorImage,
    },
    Failed(String),
    /// The user closed the file panel.
    Cancelled,
}

#[derive(Default)]
pub(super) struct ContentFlow {
    /// What waits to be placed by a click while its tool is armed.
    pub pending: Option<Content>,
    /// The waiting picture arrived by a drop or a paste: place it without a click.
    pub place_now: bool,
    /// A picture to put in place of a drawing's: `(chart, drawing, picture)`.
    pub replace_with: Option<(u64, u64, ImageContent)>,
    /// The sheet is open.
    sheet: bool,
    /// The tool the last frame had, to notice a content tool being armed.
    pub(super) last_tool: Tool,
    inbox: Option<Receiver<Outcome>>,
    target: Option<Target>,
    /// Why the last picture was refused, for the sheet.
    error: Option<String>,
    /// The Post tool: its sheets and the preview fetch.
    pub links: super::links::LinkFlow,
}

/// What the chart frames are told about content this frame: a copy taken before they run, so
/// they need nothing of the workspace while they draw.
pub(super) struct PictureWork {
    pending: Option<Content>,
    place_now: bool,
    focused: Option<u64>,
    replace_with: Option<(u64, u64, ImageContent)>,
}

impl PictureWork {
    /// What chart `chart_id` is told: the waiting picture, whether to place it now (only on the
    /// focused chart), and a picture to swap into one of its drawings.
    pub(super) fn for_chart(
        &self,
        chart_id: u64,
    ) -> (Option<Content>, bool, Option<(u64, ImageContent)>) {
        let focused = self.focused.is_none_or(|focused| focused == chart_id);
        (
            self.pending.clone(),
            self.place_now && focused,
            self.replace_with
                .as_ref()
                .filter(|(chart, ..)| *chart == chart_id)
                .map(|(_, drawing, content)| (*drawing, content.clone())),
        )
    }
}

impl ContentFlow {
    pub(super) fn work_snapshot(&self, focused: Option<u64>) -> PictureWork {
        PictureWork {
            pending: self.pending.clone(),
            place_now: self.place_now,
            focused,
            replace_with: self.replace_with.clone(),
        }
    }

    pub(super) fn sheet_open(&self) -> bool {
        self.sheet || self.links.open()
    }

    fn working(&self) -> bool {
        self.inbox.is_some()
    }
}

/// The decoded pictures the drawings show, as far as they are loaded: what a snapshot is handed,
/// frozen on the UI thread, since it has no store to ask. A picture not loaded is painted as the
/// chart paints it, as a placeholder.
pub(super) fn snapshot_images(
    ctx: &egui::Context,
    drawings: &[crate::model::Drawing],
) -> std::collections::HashMap<String, std::sync::Arc<ColorImage>> {
    let Some(host) = asset_host::host(ctx) else {
        return Default::default();
    };
    drawings
        .iter()
        .filter_map(|drawing| match drawing.geometry() {
            Some(crate::model::Geometry::Image(image)) => Some(image.asset.clone()),
            // A card's author picture is a stored picture too.
            Some(crate::model::Geometry::Card { card, .. }) => {
                card.meta.as_ref().and_then(|meta| meta.avatar.clone())
            }
            _ => None,
        })
        .filter_map(|asset| host.ready(&asset).map(|pixels| (asset, pixels)))
        .collect()
}

/// A picture's name for lists: the file's name without its extension, cut to what a name may be.
pub(super) fn display_name(file_name: &str) -> String {
    let stem = std::path::Path::new(file_name)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or(file_name);
    stem.trim().chars().take(MAX_NAME_CHARS).collect()
}

/// Reads, checks, stores and decodes one picture. Runs on a worker.
fn work(source: Source, store: &SharedStore) -> Outcome {
    let (bytes, name) = match source {
        Source::Panel => match file_dialog::choose_image("Choose a picture") {
            Ok(Some(path)) => match image_ingest::read_file(&path) {
                Ok(bytes) => (bytes, display_name(&path.to_string_lossy())),
                Err(error) => return Outcome::Failed(error.message().to_owned()),
            },
            Ok(None) => return Outcome::Cancelled,
            Err(message) => return Outcome::Failed(message),
        },
        Source::Clipboard => match crate::native_bridge::clipboard_picture() {
            Some(bytes) => (bytes, "Pasted image".to_owned()),
            None => return Outcome::Failed("There is no picture on the clipboard.".to_owned()),
        },
        Source::Path(path) => match image_ingest::read_file(&path) {
            Ok(bytes) => (bytes, display_name(&path.to_string_lossy())),
            Err(error) => return Outcome::Failed(error.message().to_owned()),
        },
    };
    let picture = match image_ingest::ingest(&bytes) {
        Ok(picture) => picture,
        Err(error) => return Outcome::Failed(error.message().to_owned()),
    };
    let Some(pixels) = image_ingest::pixels(&picture.bytes) else {
        return Outcome::Failed(IngestError::Corrupt.message().to_owned());
    };
    let stored = store
        .lock()
        .map_err(|_| "The library is busy.".to_owned())
        .and_then(|store| {
            store
                .put_asset(&StoredAsset::from_ingested(&picture))
                .map_err(|error| format!("The picture could not be saved: {error}"))
        });
    if let Err(message) = stored {
        return Outcome::Failed(message);
    }
    Outcome::Stored {
        content: ImageContent::new(picture.id, name, [picture.width, picture.height]),
        pixels,
    }
}

impl ChartWorkspace {
    /// Starts reading a picture on a worker, for `target`. One at a time.
    pub(super) fn start_picture(&mut self, ctx: &egui::Context, source: Source, target: Target) {
        if self.content.working() {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        self.content.inbox = Some(receiver);
        self.content.target = Some(target);
        self.content.error = None;
        let (store, ctx) = (self.store.clone(), ctx.clone());
        work_queue::ASSETS.submit(move || {
            let _ = sender.send(work(source, &store));
            ctx.request_repaint();
        });
    }

    /// Asks for a picture to replace the one drawing `drawing_id` of chart `chart_id` shows.
    pub(super) fn replace_picture_request(
        &mut self,
        ctx: &egui::Context,
        chart_id: u64,
        drawing_id: u64,
    ) {
        self.start_picture(
            ctx,
            Source::Panel,
            Target::Replace {
                chart_id,
                drawing_id,
            },
        );
    }

    /// Everything the content tools need each frame: the sheet, what was read, drops.
    pub(super) fn tick_content(&mut self, ctx: &egui::Context) {
        self.take_picture(ctx);
        let tool = self.active_tool;
        let content_tool = matches!(tool, Tool::Image | Tool::Post);
        let armed = content_tool && self.content.last_tool != tool;
        self.content.last_tool = tool;
        if !content_tool {
            // The tool was put down: what waited for it, and its sheets, go.
            if !self.content.working() {
                self.content.pending = None;
                self.content.sheet = false;
            }
            self.content.links.close_sheet();
        } else if armed {
            // Whatever waited for another content tool is dropped; this one asks again.
            self.content.pending = None;
            match tool {
                Tool::Post => self.content.links.start(),
                _ => {
                    self.content.sheet = true;
                    self.content.links.close_sheet();
                }
            }
            self.content.error = None;
        }
        self.tick_links(ctx);
        self.take_drops(ctx);
        if self.content.sheet {
            self.show_image_sheet(ctx);
        }
    }

    /// The picture a worker finished, if one did.
    fn take_picture(&mut self, ctx: &egui::Context) {
        let Some(inbox) = &self.content.inbox else {
            return;
        };
        let outcome = match inbox.try_recv() {
            Ok(outcome) => outcome,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => {
                Outcome::Failed("The picture could not be read.".to_owned())
            }
        };
        self.content.inbox = None;
        let target = self.content.target.take();
        match outcome {
            Outcome::Cancelled => {}
            Outcome::Failed(message) => {
                if self.content.sheet {
                    self.content.error = Some(message);
                } else {
                    self.say(message, true);
                }
            }
            Outcome::Stored { content, pixels } => {
                if let Some(host) = asset_host::host(ctx) {
                    host.insert_ready(&content.asset, pixels);
                }
                self.content.sheet = false;
                match target {
                    Some(Target::Replace {
                        chart_id,
                        drawing_id,
                    }) => {
                        // A dialog open on the drawing takes the picture into its draft, so OK
                        // commits it with the rest and Cancel leaves the drawing as it was.
                        match self.drawing_props.as_mut() {
                            Some(dialog)
                                if dialog.chart_id == chart_id
                                    && dialog.draft().id() == drawing_id =>
                            {
                                dialog.replace_picture(&content);
                            }
                            _ => self.content.replace_with = Some((chart_id, drawing_id, content)),
                        }
                    }
                    Some(Target::Place { at_once }) => {
                        self.content.pending = Some(Content::Image(content));
                        self.content.place_now = at_once;
                        // The tool is armed so the picture follows the pointer if it is not
                        // placed at once.
                        self.active_tool = Tool::Image;
                        self.content.last_tool = Tool::Image;
                    }
                    None => {}
                }
            }
        }
    }

    /// Pictures dropped on the window.
    fn take_drops(&mut self, ctx: &egui::Context) {
        let dropped = ctx.input(|input| input.raw.dropped_files.clone());
        let Some(file) = dropped.into_iter().next() else {
            return;
        };
        // Read by path, with a limit on how much, never by asking the platform for all of it.
        let source = Source::Path(file.path().to_path_buf());
        self.start_picture(ctx, source, Target::Place { at_once: true });
    }

    fn show_image_sheet(&mut self, ctx: &egui::Context) {
        let dragging = ctx.input(|input| !input.raw.hovered_files.is_empty());
        let action = content_sheet::show_image(
            ctx,
            &ImageSheet {
                working: self.content.working(),
                error: self.content.error.as_deref(),
                dragging,
            },
        );
        match action {
            ImageSheetAction::None => {}
            ImageSheetAction::Choose => {
                self.start_picture(ctx, Source::Panel, Target::Place { at_once: false });
            }
            ImageSheetAction::Paste => {
                self.start_picture(ctx, Source::Clipboard, Target::Place { at_once: true });
            }
            ImageSheetAction::Close => {
                self.content.sheet = false;
                self.content.error = None;
                self.active_tool = Tool::Cursor;
            }
        }
    }

    /// A chart said its waiting picture was placed.
    pub(super) fn picture_used(&mut self) {
        self.content.pending = None;
        self.content.place_now = false;
    }

    /// A frame has run: a replacement offered to the chart has been seen by it.
    pub(super) fn picture_work_done(&mut self) {
        self.content.replace_with = None;
        self.content.place_now = false;
    }

    /// Copies the picture named `asset` to the clipboard, when it is loaded.
    pub(super) fn copy_picture(&mut self, ctx: &egui::Context, asset: &str) {
        match asset_host::host(ctx).and_then(|host| host.ready(asset)) {
            Some(pixels) => {
                ctx.copy_image((*pixels).clone());
                self.say("Copied the image.", false);
            }
            None => self.say("That image is not available to copy yet.", true),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_name_becomes_a_picture_name_without_its_folder_or_extension() {
        assert_eq!(
            display_name("/Users/me/Charts/levels.final.png"),
            "levels.final"
        );
        assert_eq!(display_name("photo.JPG"), "photo");
        assert_eq!(display_name("  spaced name .png"), "spaced name");
        assert_eq!(
            display_name(&"x".repeat(500)).chars().count(),
            MAX_NAME_CHARS
        );
        assert_eq!(display_name(""), "");
    }
}
