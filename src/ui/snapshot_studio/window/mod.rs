//! The Studio's window: a modal that previews the image, restyles and reframes it, and exports
//! it. Immediate mode: [`show`] is given what the workspace knows ([`Input`]) and answers with
//! what the user did ([`Action`]s) and how much room the preview had ([`Output`]); it keeps no
//! state of its own beyond what egui remembers (focus, a drag in progress), makes no thread and
//! never touches the configuration. The workspace applies the actions and draws again.
//!
//! Layout, top to bottom: the title and the close button; the controls in groups (Area, Style,
//! Size, Background, Include), wrapped onto a second line where the window is too narrow; the
//! preview in a rounded well; the Width and Height rulers with Reset view; and the footer with
//! the image's summary and the two ways to export it. The window is as wide as the screen allows
//! up to [`MAX_WIDTH`] and as tall as the screen allows; the preview takes what the rest leaves.

mod footer;
mod preview;
mod swatches;
mod toolbar;

use crate::model::chart::snapshot::framing::{Dial, Step};
use crate::model::chart::snapshot::studio::{
    Area, AspectRatio, Background, Include, StudioConfig, Style,
};
use crate::ui::icons::Icon;
use crate::ui::kit::{self, SEMIBOLD, sheet, size};
use crate::ui::theme;
use eframe::egui::{self, Id, Key, Modifiers, TextureId, Vec2, pos2, vec2};

/// The widest the window gets, in points.
pub const MAX_WIDTH: f32 = 1120.0;
/// The tallest the window gets, in points.
pub const MAX_HEIGHT: f32 = 900.0;
/// The share of the screen the window may take across and down.
const SCREEN_SHARE: f32 = 0.92;
/// The margin inside the window at its left and right.
pub const PAD: f32 = 20.0;
/// The least the preview is given, however small the screen.
const MIN_PREVIEW: f32 = 140.0;
const HEADER_HEIGHT: f32 = 52.0;

/// How the image leaves the Studio.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Export {
    Download,
    CopyImage,
}

/// Something the user did that the workspace is to carry out.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    /// The close button or Escape.
    Close,
    /// An area picked. Portion goes back to the chart to select (also when it is already the
    /// area); the others take a new picture of the live workspace.
    SetArea(Area),
    SetStyle(Style),
    SetSize(AspectRatio),
    SetBackground(Background),
    SetInclude(Include),
    /// A dial dragged to a value.
    DragDial(Dial, f64),
    /// A dial's drag ended: near 1× it lands on 1×.
    ReleaseDial,
    /// A key moved a dial one step up (`sign` positive) or down.
    StepDial(Dial, Step, i8),
    ResetDial(Dial),
    /// The preview dragged: how far the pointer has travelled since it went down, in points of
    /// the image (not of the screen).
    DragPreview(Vec2),
    ReleasePreview,
    /// An arrow key on the preview: the picture moves one step across (positive: right) and down
    /// (positive: down).
    NudgePreview {
        across: i8,
        down: i8,
    },
    /// Home, 0 or a double-click on the preview: the position goes back, the dials stay.
    ResetPosition,
    /// Reset view: the dials and the position.
    ResetView,
    Export(Export),
}

impl Action {
    /// Whether the action reframes the picture (the dials, the drag and Reset view), which only
    /// a single chart can have.
    pub fn is_framing(&self) -> bool {
        matches!(
            self,
            Self::DragDial(..)
                | Self::ReleaseDial
                | Self::StepDial(..)
                | Self::ResetDial(_)
                | Self::DragPreview(_)
                | Self::ReleasePreview
                | Self::NudgePreview { .. }
                | Self::ResetPosition
                | Self::ResetView
        )
    }
}

/// A picture of the image as the pipeline last made it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Image {
    pub texture: TextureId,
    /// Its size in pixels.
    pub pixels: [usize; 2],
    /// The size in points of the composition it shows, which is what a drag on it is measured in
    /// (the image may be from before the last change, so this is its own).
    pub composition: Vec2,
}

/// What stands in the preview.
#[derive(Clone, Copy, Debug)]
pub struct Preview<'a> {
    /// The latest picture; none until the first is made.
    pub image: Option<Image>,
    /// Why there will be none, in words; none while it is simply being made.
    pub failure: Option<&'a str>,
}

/// What the workspace tells the window each frame.
pub struct Input<'a> {
    pub config: &'a StudioConfig,
    /// The areas on offer, in order.
    pub areas: &'a [Area],
    pub has_indicators: bool,
    pub has_drawings: bool,
    /// The dials and the drag act on this capture: one chart, not an arrangement of several.
    pub reframable: bool,
    pub preview: Preview<'a>,
    /// The summary line of the footer, or why there is no image to describe.
    pub summary: Result<&'a str, &'a str>,
    /// An export of this window's is being made, and which.
    pub exporting: Option<Export>,
    pub can_export: bool,
}

/// What a frame of the window reports.
#[derive(Debug, Default)]
pub struct Output {
    pub actions: Vec<Action>,
    /// A drag of the preview or of a ruler is under way: the preview may be drawn coarser until
    /// it ends.
    pub interacting: bool,
    /// The room the preview image may take, in pixels: what to draw it at to be crisp.
    pub preview_pixels: (u32, u32),
}

/// The window's size on a screen of `screen` points.
pub fn window_size(screen: Vec2) -> Vec2 {
    vec2(
        (screen.x * SCREEN_SHARE).min(MAX_WIDTH),
        (screen.y * SCREEN_SHARE).min(MAX_HEIGHT),
    )
}

/// Draws the window and says what the user did.
pub fn show(ctx: &egui::Context, id: Id, input: &Input<'_>) -> Output {
    let size = window_size(ctx.content_rect().size());
    let mut output = Output::default();
    sheet::show(ctx, id, size.x, |ui| {
        let top = ui.cursor().top();
        header(ui, size.x, &mut output);
        toolbar::show(ui, input, size.x, &mut output);
        let used = ui.cursor().top() - top;
        let preview_height =
            (size.y - used - footer::FRAMING_HEIGHT - footer::BAR_HEIGHT).max(MIN_PREVIEW);
        preview::show(ui, input, size.x, preview_height, &mut output);
        footer::framing(ui, input, size.x, &mut output);
        footer::bar(ui, input, size.x, &mut output);
    });
    if ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape)) {
        output.actions.push(Action::Close);
    }
    output
}

fn header(ui: &mut egui::Ui, width: f32, output: &mut Output) {
    let (row, _) = ui.allocate_exact_size(vec2(width, HEADER_HEIGHT), egui::Sense::hover());
    let title = kit::galley(
        ui,
        "Studio",
        size::TITLE2,
        SEMIBOLD,
        theme::text_primary(),
        f32::INFINITY,
    );
    ui.painter().galley(
        pos2(row.left() + PAD, row.center().y - title.size().y / 2.0),
        title,
        theme::text_primary(),
    );
    let close = egui::Rect::from_center_size(
        pos2(row.right() - PAD + 4.0 - 14.0, row.center().y),
        vec2(28.0, 28.0),
    );
    if kit::hover_button(
        ui,
        close,
        Id::new("studio-close"),
        Icon::Xmark,
        "Close",
        true,
        true,
    )
    .clicked()
    {
        output.actions.push(Action::Close);
    }
}

#[cfg(test)]
mod gallery_tests;
#[cfg(test)]
mod rig;
#[cfg(test)]
mod tests;
