//! A short message over the bottom of the window: what a chart command did, why it did not, or
//! that something is still going on. It never takes the pointer, and the workspace that raised it
//! drops it after a few seconds (a progress notice stays for as long as the work does).

use crate::ui::icons::{self, Icon};
use crate::ui::kit::{self, REGULAR, size};
use crate::ui::theme;
use eframe::egui::{self, Align2, Context, Id, Margin, Order, Ui, vec2};

/// What leads the message.
enum Lead {
    Icon(Icon, egui::Color32),
    /// An animated spinner: work is in progress.
    Spinner,
}

/// A notice for what a chart command did (a check) or why it did not (a warning).
pub fn show(ctx: &Context, id: Id, text: &str, error: bool) {
    let lead = if error {
        Lead::Icon(Icon::TriangleFill, kit::ORANGE)
    } else {
        Lead::Icon(Icon::Check, theme::text_secondary())
    };
    area(ctx, id, text, lead);
}

/// A notice for work that is still going on: a spinner, so it is never taken for a result. It
/// keeps the window repainting for as long as it is shown, so the spinner turns smoothly.
pub fn show_progress(ctx: &Context, id: Id, text: &str) {
    area(ctx, id, text, Lead::Spinner);
    ctx.request_repaint();
}

fn area(ctx: &Context, id: Id, text: &str, lead: Lead) {
    egui::Area::new(id)
        .order(Order::Foreground)
        .anchor(Align2::CENTER_BOTTOM, vec2(0.0, -32.0))
        .interactable(false)
        .show(ctx, |ui| {
            kit::menu_frame()
                .inner_margin(Margin::symmetric(14, 9))
                .show(ui, |ui| content(ui, text, lead));
        });
}

fn content(ui: &mut Ui, text: &str, lead: Lead) {
    ui.horizontal(|ui| {
        match lead {
            Lead::Icon(icon, color) => {
                icons::inline(ui, icon, 12.0, color);
            }
            Lead::Spinner => {
                ui.add(
                    egui::Spinner::new()
                        .size(12.0)
                        .color(theme::text_secondary()),
                );
            }
        }
        let response = ui.label(kit::text(text, size::BODY, REGULAR, theme::text_primary()));
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, text));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(ctx: &Context, body: impl FnMut(&Context)) -> egui::FullOutput {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(600.0, 400.0),
            )),
            ..Default::default()
        };
        let mut body = body;
        let mut out = ctx.run_ui(raw, |ui| body(ui.ctx()));
        out.textures_delta.clear();
        out
    }

    fn id() -> Id {
        Id::new("notice-test")
    }

    /// How soon a notice asks for the next frame once it has settled (a new area is laid out
    /// over a few frames first).
    fn settled_repaint(show: impl Fn(&Context)) -> std::time::Duration {
        let ctx = Context::default();
        let mut out = None;
        for step in 0..8 {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(600.0, 400.0),
                )),
                time: Some(f64::from(step) * 0.1),
                ..Default::default()
            };
            let mut frame = ctx.run_ui(raw, |ui| show(ui.ctx()));
            frame.textures_delta.clear();
            out = Some(frame);
        }
        assert!(
            ctx.memory(|memory| memory.area_rect(id())).is_some(),
            "the notice is drawn"
        );
        out.expect("frames")
            .viewport_output
            .values()
            .map(|viewport| viewport.repaint_delay)
            .min()
            .expect("a viewport")
    }

    #[test]
    fn a_progress_notice_keeps_the_window_painting_so_the_spinner_turns() {
        let delay = settled_repaint(|ctx| show_progress(ctx, id(), "Uploading to ImgBB…"));
        assert!(delay < std::time::Duration::from_millis(100), "{delay:?}");
    }

    #[test]
    fn a_result_notice_does_not_keep_the_window_repainting() {
        let delay = settled_repaint(|ctx| show(ctx, id(), "Link copied", false));
        assert!(
            delay >= std::time::Duration::from_millis(100),
            "a message that is finished needs no animation: {delay:?}"
        );
    }

    #[test]
    fn a_progress_notice_leaves_the_pointer_to_the_window_below() {
        let ctx = Context::default();
        frame(&ctx, |ctx| show_progress(ctx, id(), "Uploading to ImgBB…"));
        // Where the notice sits, the pointer is still the window's.
        let rect = ctx.memory(|memory| memory.area_rect(id())).expect("drawn");
        assert!(
            !ctx.layer_id_at(rect.center())
                .is_some_and(|layer| layer.id == id()),
            "an interactable(false) area is never picked by the pointer"
        );
    }

    /// The notices in both themes: an upload in progress (fast and slow), then a result and a
    /// warning for comparison. `DEGENVIEW_GALLERY_DIR=<dir> cargo test --lib
    /// gallery_of_the_notices -- --ignored` writes them.
    #[test]
    #[ignore = "writes images for a person to look at"]
    fn gallery_of_the_notices() {
        use crate::ui::render_tests::{Screen, gallery_dir};
        use crate::ui::theme::{self, AppTheme};
        if gallery_dir().is_none() {
            return;
        }
        let _restore = theme::RestoreDefaultTheme;
        let cases: [(&str, fn(&Context)); 4] = [
            ("notice-progress", |ctx| {
                show_progress(ctx, Id::new("n"), "Uploading to ImgBB…");
            }),
            ("notice-progress-slow", |ctx| {
                show_progress(
                    ctx,
                    Id::new("n"),
                    "Still uploading to ImgBB… a slow connection can take a while",
                );
            }),
            ("notice-result", |ctx| {
                show(ctx, Id::new("n"), "Link copied", false);
            }),
            ("notice-error", |ctx| {
                show(
                    ctx,
                    Id::new("n"),
                    "Could not reach ImgBB. Check your connection.",
                    true,
                );
            }),
        ];
        for (suffix, app_theme) in [("dark", AppTheme::Dark), ("light", AppTheme::Light)] {
            for (name, notice) in cases {
                let mut screen = Screen::new(egui::vec2(560.0, 120.0));
                theme::apply(&screen.ctx, app_theme);
                let mut last = None;
                for _ in 0..4 {
                    last = Some(screen.run(Vec::new(), |ui| notice(ui.ctx())));
                }
                screen.save(&format!("{name}-{suffix}"), last.expect("a frame"));
            }
        }
    }
}
