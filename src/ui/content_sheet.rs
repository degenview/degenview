//! The sheet that opens when the Image tool is armed: where a picture is chosen. It offers the
//! system's file panel and the clipboard, and says it can be dropped on the chart instead. It
//! only asks: the workspace reads the picture, checks it and stores it (off the UI thread), and the
//! sheet shows what it is doing and why a picture was refused.

use crate::ui::icons::{self, Icon};
use crate::ui::kit::{self, REGULAR, size};
use crate::ui::{sheet, theme};
use eframe::egui::{self, Align2, Id, Key, Rect, Sense, Stroke, vec2};

const WIDTH: f32 = 420.0;

/// What the user asked of the image sheet this frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImageSheetAction {
    None,
    /// Open the file panel.
    Choose,
    /// Take the picture on the clipboard.
    Paste,
    /// Close the sheet and put the tool down.
    Close,
}

/// What the sheet has to say.
pub struct ImageSheet<'a> {
    /// A picture is being read: nothing else can be started.
    pub working: bool,
    /// Why the last picture was refused.
    pub error: Option<&'a str>,
    /// Whether a drag with files is over the window right now.
    pub dragging: bool,
}

/// Shows the sheet.
pub fn show_image(ctx: &egui::Context, view: &ImageSheet) -> ImageSheetAction {
    let mut action = ImageSheetAction::None;
    sheet::show(ctx, Id::new("image-sheet"), WIDTH, |ui| {
        ui.add_space(f32::from(sheet::INSET));
        sheet::inset(ui, |ui| {
            sheet::header(
                ui,
                Icon::ImageTool,
                "Add image",
                "A PNG or JPEG, scaled to fit and kept with your drawings",
                kit::ACCENT,
            );
            ui.add_space(18.0);
            drop_zone(ui, view);
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                if kit::push_button(ui, Some(Icon::ImageTool), "Choose file…", !view.working)
                    .clicked()
                {
                    action = ImageSheetAction::Choose;
                }
                if kit::push_button(
                    ui,
                    Some(Icon::Clipboard),
                    "Paste from clipboard",
                    !view.working,
                )
                .clicked()
                {
                    action = ImageSheetAction::Paste;
                }
            });
            ui.add_space(10.0);
            let (text, color) = match (view.working, view.error) {
                (true, _) => (Some("Reading the picture…"), theme::text_secondary()),
                (false, Some(error)) => (Some(error), kit::RED),
                (false, None) => (None, theme::text_secondary()),
            };
            if let Some(text) = text {
                ui.label(kit::text(text, size::SUBHEADLINE, REGULAR, color));
            }
        });
        ui.add_space(18.0);
        sheet::footer(ui, |ui| {
            if kit::sheet_button(ui, "Cancel", false, true).clicked() {
                action = ImageSheetAction::Close;
            }
        });
    });
    if !view.working && ctx.input(|input| input.key_pressed(Key::Escape)) {
        action = ImageSheetAction::Close;
    }
    action
}

/// A dashed box that says where a picture can be dropped.
fn drop_zone(ui: &mut egui::Ui, view: &ImageSheet) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, 96.0), Sense::hover());
    let color = if view.dragging {
        kit::ACCENT
    } else {
        theme::text_tertiary()
    };
    if view.dragging {
        ui.painter()
            .rect_filled(rect, 10.0, theme::with_alpha(kit::ACCENT, 0.10));
    }
    let path = [
        rect.left_top(),
        rect.right_top(),
        rect.right_bottom(),
        rect.left_bottom(),
    ];
    ui.painter().extend(egui::Shape::dashed_line(
        &[path[0], path[1], path[2], path[3], path[0]],
        Stroke::new(1.0, color),
        6.0,
        4.0,
    ));
    icons::paint(
        ui.painter(),
        Rect::from_center_size(rect.center() - vec2(0.0, 16.0), vec2(26.0, 26.0)),
        Icon::ImageTool,
        color,
    );
    ui.painter().text(
        rect.center() + vec2(0.0, 18.0),
        Align2::CENTER_CENTER,
        if view.dragging {
            "Drop to add the picture"
        } else {
            "Or drop a picture on the chart"
        },
        egui::FontId::proportional(13.0),
        if view.dragging {
            kit::ACCENT
        } else {
            theme::text_secondary()
        },
    );
}
