//! What the Script Manager says while the library has no scripts: the detail area, laid out like the
//! Logs and Profile tabs' introductions (a picture, what the manager is for, three numbered steps,
//! an example, and the two ways to begin), and the sidebar's short form of the same.

use super::Cmd;
use crate::ui::icons::Icon;
use crate::ui::pine_logs_view::{centered, intro, step};
use crate::ui::{kit, theme};
use eframe::egui::{
    self, Color32, CursorIcon, FontId, Rect, Sense, Stroke, StrokeKind, Ui, pos2,
    text::{LayoutJob, TextFormat},
    vec2,
};

const DOCS_URL: &str = "https://www.tradingview.com/pine-script-docs/";

/// The detail area with no scripts at all.
pub(super) fn library(ui: &mut Ui, cmds: &mut Vec<Cmd>) {
    centered(ui, |ui| {
        intro(
            ui,
            "Write your own scripts",
            "Indicators and strategies in Pine Script.",
            |ui| {
                step(ui, 1, |ui| {
                    ui.label(muted("Create a script, or start from the examples."));
                });
                step(ui, 2, |ui| {
                    ui.label(muted("Edit it; problems are flagged as you type."));
                });
                step(ui, 3, |ui| {
                    ui.label(muted("Preview it, then add it to any chart."));
                });
            },
            example,
            picture,
            |ui| {
                ui.add_space(14.0);
                ui.spacing_mut().item_spacing.y = 8.0;
                if kit::push_button(ui, Some(Icon::Plus), "Create Your First Script", true)
                    .clicked()
                {
                    cmds.push(Cmd::NewScript);
                }
                if kit::push_button(ui, Some(Icon::Download), "Add Example Scripts", true).clicked()
                {
                    cmds.push(Cmd::AddExamples);
                }
                ui.add_space(6.0);
                links(ui);
            },
        );
    });
}

/// The sidebar's list with no scripts: the picture, two lines, and the examples. The New Script
/// button is already under the list.
pub(super) fn sidebar(ui: &mut Ui, cmds: &mut Vec<Cmd>) {
    ui.add_space(26.0);
    ui.vertical_centered(|ui| {
        ui.set_max_width((ui.available_width() - 40.0).max(80.0));
        let (rect, _) = ui.allocate_exact_size(vec2(96.0, 84.0), Sense::hover());
        picture(ui, rect);
        ui.add_space(12.0);
        ui.label(kit::text(
            "No scripts yet",
            kit::size::HEADLINE,
            kit::SEMIBOLD,
            theme::text_primary(),
        ));
        ui.add_space(3.0);
        ui.label(muted(
            "Create one with the button below, or start from the examples.",
        ));
        ui.add_space(12.0);
        if kit::push_button(ui, Some(Icon::Download), "Add Example Scripts", true).clicked() {
            cmds.push(Cmd::AddExamples);
        }
    });
    ui.add_space(12.0);
}

fn muted(text: &str) -> egui::RichText {
    kit::text(
        text,
        kit::size::SUBHEADLINE,
        kit::REGULAR,
        theme::text_secondary(),
    )
}

/// The documentation and the community scripts, side by side when the column has room and stacked
/// when it has not.
fn links(ui: &mut Ui) {
    let items = [
        ("Pine Script documentation", DOCS_URL),
        ("Community scripts", super::sidebar::COMMUNITY_URL),
    ];
    let widths: Vec<f32> = items
        .iter()
        .map(|(label, _)| {
            kit::galley(
                ui,
                label,
                kit::size::SUBHEADLINE,
                kit::MEDIUM,
                theme::text_secondary(),
                f32::INFINITY,
            )
            .size()
            .x
        })
        .collect();
    let gap = 14.0;
    let side_by_side = widths.iter().sum::<f32>() + gap <= ui.available_width();
    let link = |ui: &mut Ui, label: &str, url: &str| {
        let response = ui
            .add(
                egui::Label::new(
                    kit::text(
                        label,
                        kit::size::SUBHEADLINE,
                        kit::MEDIUM,
                        theme::text_secondary(),
                    )
                    .underline(),
                )
                .sense(Sense::click()),
            )
            .on_hover_cursor(CursorIcon::PointingHand)
            .on_hover_text(url);
        if response.clicked() {
            ui.ctx().open_url(egui::OpenUrl::new_tab(url));
        }
    };
    if side_by_side {
        let total = widths.iter().sum::<f32>() + gap;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            ui.add_space(((ui.available_width() - total) / 2.0).max(0.0));
            for (label, url) in items {
                link(ui, label, url);
            }
        });
    } else {
        for (label, url) in items {
            link(ui, label, url);
        }
    }
}

/// A script on a soft disc: the first lines of a Pine file in the colours the editor gives a
/// keyword, a call and a variable.
pub(super) fn picture(ui: &Ui, rect: Rect) {
    let painter = ui.painter();
    painter.circle_filled(rect.center(), 40.0, kit::ACCENT.gamma_multiply(0.10));
    let card = Rect::from_center_size(rect.center(), vec2(64.0, 54.0));
    painter.rect_filled(card, 9.0, theme::chrome());
    painter.rect_stroke(
        card,
        9.0,
        Stroke::new(1.5, theme::text_tertiary()),
        StrokeKind::Inside,
    );
    let lines = [
        (theme::syntax::keyword(), 0.0, 0.46),
        (theme::syntax::function(), 0.0, 0.74),
        (theme::syntax::variable(), 8.0, 0.42),
    ];
    for (index, (color, indent, length)) in lines.into_iter().enumerate() {
        let y = card.top() + 14.0 + index as f32 * 13.0;
        let left = card.left() + 11.0 + indent;
        painter.hline(
            left..=left + (card.width() - 22.0 - indent) * length,
            y,
            Stroke::new(2.2, color.gamma_multiply(0.8)),
        );
    }
}

/// The declaration every script starts with, in the editor's colours, on a well.
fn example(ui: &mut Ui, width: f32) {
    let mono = |color: Color32| TextFormat {
        font_id: FontId::monospace(11.0),
        color,
        ..Default::default()
    };
    let mut job = LayoutJob::default();
    job.append("indicator", 0.0, mono(theme::syntax::function()));
    job.append("(", 0.0, mono(theme::text_secondary()));
    job.append("\"My Script\"", 0.0, mono(theme::syntax::string()));
    job.append(", ", 0.0, mono(theme::text_secondary()));
    job.append("overlay", 0.0, mono(theme::syntax::variable()));
    job.append(" = ", 0.0, mono(theme::text_secondary()));
    job.append("true", 0.0, mono(theme::syntax::keyword()));
    job.append(")", 0.0, mono(theme::text_secondary()));
    let galley = ui.painter().layout_job(job);
    let (rect, _) = ui.allocate_exact_size(vec2(width, galley.size().y + 18.0), Sense::hover());
    ui.painter().rect_filled(rect, 8.0, theme::well_fill());
    ui.painter().galley(
        pos2(rect.center().x - galley.size().x / 2.0, rect.top() + 9.0),
        galley,
        Color32::PLACEHOLDER,
    );
}

#[cfg(test)]
mod tests {
    use super::super::ScriptManager;
    use super::super::store::ScriptStore;
    use crate::store::Store;
    use crate::ui::render_tests::{Screen, gallery_dir};
    use crate::ui::theme::{self, AppTheme};
    use eframe::egui;
    use std::sync::{Arc, Mutex};

    fn empty_manager(tag: &str) -> (ScriptManager, std::path::PathBuf) {
        let root =
            std::env::temp_dir().join(format!("degenview-empty-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let shared = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
        let store = ScriptStore::new(root.join("Scripts"), root.join("ScriptMetadata"));
        let mut manager = ScriptManager::with_script_store(1, shared, Some(store));
        manager.loaded = true;
        manager.refresh();
        (manager, root)
    }

    fn frames(manager: &mut ScriptManager, screen: &mut Screen, count: usize) -> egui::FullOutput {
        let mut last = None;
        for _ in 0..count {
            theme::sync(&screen.ctx);
            last = Some(screen.run(Vec::new(), |ui| {
                manager.ui(ui, None);
            }));
        }
        last.expect("a frame")
    }

    /// An empty library draws at a wide and a small window without panicking, and the buttons ask
    /// for what they say.
    #[test]
    fn empty_library_draws_at_every_size() {
        let (mut manager, root) = empty_manager("sizes");
        assert!(manager.scripts.is_empty());
        for size in [
            (1360.0, 860.0),
            (900.0, 600.0),
            (640.0, 420.0),
            (460.0, 300.0),
        ] {
            let mut screen = Screen::new(egui::vec2(size.0, size.1));
            theme::apply(&screen.ctx, AppTheme::Dark);
            frames(&mut manager, &mut screen, 4);
        }
        let _ = std::fs::remove_dir_all(root);
    }

    /// Run by hand with `DEGENVIEW_GALLERY_DIR` set.
    #[test]
    #[ignore = "writes images for a person to look at"]
    fn gallery_of_the_empty_library() {
        if gallery_dir().is_none() {
            return;
        }
        for (name, app_theme) in [("dark", AppTheme::Dark), ("light", AppTheme::Light)] {
            let (mut manager, root) = empty_manager(name);
            for (size, label) in [
                ((1360.0, 860.0), "wide"),
                ((700.0, 520.0), "narrow"),
                ((460.0, 330.0), "small"),
            ] {
                let mut screen = Screen::new(egui::vec2(size.0, size.1));
                theme::apply(&screen.ctx, app_theme);
                let output = frames(&mut manager, &mut screen, 6);
                screen.save(&format!("empty-library-{label}-{name}"), output);
            }
            manager.search = "zzz".to_owned();
            let mut screen = Screen::new(egui::vec2(1000.0, 640.0));
            theme::apply(&screen.ctx, app_theme);
            let output = frames(&mut manager, &mut screen, 6);
            screen.save(&format!("empty-library-search-{name}"), output);
            let _ = std::fs::remove_dir_all(root);
        }
    }
}
