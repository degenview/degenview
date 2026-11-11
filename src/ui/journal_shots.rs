//! The screenshots on a trade's review: a strip of thumbnails with Attach and Paste, and a
//! full-size viewer. A pure view: it is handed what each picture is right now and returns what the
//! user asked for; reading, storing and decoding happen elsewhere.

use eframe::egui::{self, Align2, Color32, CursorIcon, Rect, Sense, TextureId, Ui, pos2, vec2};

use super::icons::{self, Icon};
use super::kit::{self, MEDIUM, REGULAR, size};
use super::theme;
use degenview_journal::reviews::MAX_IMAGES;

const THUMB: egui::Vec2 = egui::vec2(112.0, 72.0);

/// What one screenshot is as far as showing it goes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Thumb {
    Ready {
        texture: TextureId,
        size: [usize; 2],
    },
    Loading,
    /// Not in the store: it may be on its way from another Mac.
    Missing,
    Corrupt,
}

#[derive(Clone, Debug)]
pub struct Shot {
    pub hash: String,
    pub thumb: Thumb,
}

/// What the user asked for this frame.
#[derive(Debug, Default, PartialEq)]
pub struct Action {
    pub attach: bool,
    pub paste: bool,
    pub open: Option<String>,
    pub remove: Option<String>,
}

/// The section: thumbnails, then the two ways to add one. `busy` is a picture being read;
/// `error` the last failure.
pub fn strip(ui: &mut Ui, shots: &[Shot], busy: bool, error: Option<&str>) -> Action {
    let mut action = Action::default();
    if !shots.is_empty() {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
            for shot in shots {
                thumbnail(ui, shot, &mut action);
            }
        });
        ui.add_space(8.0);
    }
    let room = shots.len() < MAX_IMAGES;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let enabled = room && !busy;
        if kit::push_button(ui, Some(Icon::ImageTool), "Attach image\u{2026}", enabled).clicked() {
            action.attach = true;
        }
        if kit::push_button(ui, None, "Paste", enabled).clicked() {
            action.paste = true;
        }
        if busy {
            ui.label(kit::text(
                "Adding\u{2026}",
                size::CAPTION + 1.0,
                REGULAR,
                theme::text_tertiary(),
            ));
        } else if !room {
            ui.label(kit::text(
                format!("At most {MAX_IMAGES} per trade"),
                size::CAPTION + 1.0,
                REGULAR,
                theme::text_tertiary(),
            ));
        }
    });
    if let Some(error) = error {
        ui.add_space(4.0);
        ui.label(kit::text(
            error,
            size::CAPTION + 1.0,
            REGULAR,
            super::journal_kit::Sign::Loss.text(),
        ));
    }
    action
}

fn thumbnail(ui: &mut Ui, shot: &Shot, action: &mut Action) {
    let (rect, response) = ui.allocate_exact_size(THUMB, Sense::click());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 8.0, theme::quaternary(0.45));
    match shot.thumb {
        Thumb::Ready { texture, size } => {
            let image = fit(rect.shrink(1.0), size, true);
            painter.image(
                texture,
                image,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        other => {
            let text = match other {
                Thumb::Loading => "Loading\u{2026}",
                Thumb::Missing => "Waiting to sync",
                _ => "Can't be read",
            };
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                text,
                egui::FontId::proportional(size::CAPTION),
                theme::text_tertiary(),
            );
        }
    }
    painter.rect_stroke(
        rect,
        8.0,
        egui::Stroke::new(1.0, theme::border()),
        egui::StrokeKind::Inside,
    );
    let response = response
        .on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text("Open full size");
    if response.clicked() {
        action.open = Some(shot.hash.clone());
    }
    if response.hovered() || response.has_focus() {
        let badge = Rect::from_center_size(rect.right_top() + vec2(-12.0, 12.0), vec2(20.0, 20.0));
        let remove = ui
            .interact(badge, response.id.with("remove"), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand)
            .on_hover_text("Remove from this trade");
        painter.circle_filled(
            badge.center(),
            10.0,
            theme::with_alpha(Color32::BLACK, if remove.hovered() { 0.85 } else { 0.6 }),
        );
        icons::paint(
            &painter,
            Rect::from_center_size(badge.center(), vec2(10.0, 10.0)),
            Icon::Xmark,
            Color32::WHITE,
        );
        if remove.clicked() {
            action.remove = Some(shot.hash.clone());
            action.open = None;
        }
    }
}

/// The largest rectangle of `size`'s shape inside `rect`. With `upscale` a small picture grows to
/// fit; without it a picture is never shown larger than it is.
fn fit(rect: Rect, size: [usize; 2], upscale: bool) -> Rect {
    let (w, h) = (size[0].max(1) as f32, size[1].max(1) as f32);
    let scale = if upscale {
        (rect.width() / w).min(rect.height() / h)
    } else {
        (rect.width() / w).min(rect.height() / h).min(1.0)
    };
    Rect::from_center_size(rect.center(), vec2(w * scale, h * scale))
}

/// The full-size viewer. `true` when the user closed it (Esc, a click outside, the close button).
pub fn viewer(ctx: &egui::Context, shot: &Shot) -> bool {
    let mut close = false;
    let modal = egui::Modal::new(egui::Id::new(("journal-shot-viewer", &shot.hash)))
        .backdrop_color(theme::with_alpha(Color32::BLACK, 0.72))
        .frame(egui::Frame::NONE)
        .show(ctx, |ui| {
            let screen = ui.ctx().content_rect();
            let room = vec2(
                (screen.width() - 96.0).max(200.0),
                (screen.height() - 120.0).max(160.0),
            );
            match shot.thumb {
                Thumb::Ready { texture, size } => {
                    let (rect, _) = ui.allocate_exact_size(
                        fit(Rect::from_min_size(pos2(0.0, 0.0), room), size, false).size(),
                        Sense::hover(),
                    );
                    ui.painter().image(
                        texture,
                        rect,
                        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                }
                _ => {
                    ui.label(kit::text(
                        "This picture is not available.",
                        size::BODY,
                        MEDIUM,
                        Color32::WHITE,
                    ));
                }
            }
        });
    if modal.should_close() {
        close = true;
    }
    close
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(shots: &[Shot], busy: bool, error: Option<&str>) -> Action {
        let ctx = egui::Context::default();
        theme::install_offscreen(&ctx);
        let mut action = Action::default();
        let mut out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(420.0, 300.0))),
                ..Default::default()
            },
            |ui| action = strip(ui, shots, busy, error),
        );
        out.textures_delta.clear();
        action
    }

    #[test]
    fn fit_keeps_the_shape_and_never_grows_a_viewed_picture() {
        let room = Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 200.0));
        let wide = fit(room, [800, 200], false);
        assert_eq!(wide.size(), vec2(400.0, 100.0));
        let small = fit(room, [40, 20], false);
        assert_eq!(small.size(), vec2(40.0, 20.0));
        let grown = fit(room, [40, 20], true);
        assert_eq!(grown.size(), vec2(400.0, 200.0));
    }

    #[test]
    fn the_strip_draws_empty_loading_and_broken() {
        let shot = |thumb| Shot {
            hash: "a".repeat(64),
            thumb,
        };
        for shots in [
            vec![],
            vec![shot(Thumb::Loading)],
            vec![shot(Thumb::Missing), shot(Thumb::Corrupt)],
            vec![shot(Thumb::Ready {
                texture: TextureId::default(),
                size: [30, 20],
            })],
        ] {
            assert_eq!(run(&shots, false, None), Action::default());
        }
        assert_eq!(run(&[], true, Some("nope")), Action::default());
    }
}
