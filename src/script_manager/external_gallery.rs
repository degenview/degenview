//! Pictures of a script edited outside the app: the note after a reload, and the question over
//! unsaved typing. Run by hand:
//! `DEGENVIEW_GALLERY_DIR=… cargo test --lib script_manager::external_gallery -- --ignored`.

use std::time::{Duration, Instant};

use eframe::egui;

use super::debug_tests::Rig;
use super::*;
use crate::ui::render_tests::{Screen, gallery_dir};
use crate::ui::theme::AppTheme;

const OUTSIDE: &str = "//@version=6
indicator(\"Written by a coding agent\")
length = input.int(20, \"Length\")
plot(ta.sma(close, length))
";

fn frames(rig: &mut Rig, screen: &mut Screen, count: usize) -> egui::FullOutput {
    let mut last = None;
    for _ in 0..count {
        theme::sync(&screen.ctx);
        let manager = &mut rig.manager;
        last = Some(screen.run(Vec::new(), |ui| {
            manager.ui(ui, None);
        }));
        std::thread::sleep(Duration::from_millis(15));
    }
    last.expect("a frame")
}

fn outside_edit(rig: &mut Rig, text: &str, from: Instant) {
    let id = rig.manager.open.as_ref().unwrap().id.clone();
    let path = rig
        .manager
        .script_store
        .as_mut()
        .unwrap()
        .file_path(&id)
        .unwrap()
        .unwrap();
    std::fs::write(path, text).unwrap();
    let ctx = rig.ctx.clone();
    for ms in [0, 500, 1000] {
        let now = from + Duration::from_millis(ms);
        if rig.manager.external_scan_due(now) {
            rig.manager.refresh();
            rig.manager.reconcile_open(&ctx, now);
        }
    }
}

#[test]
#[ignore = "writes images for a person to look at"]
fn gallery_of_outside_edits() {
    if gallery_dir().is_none() {
        return;
    }
    for (theme_name, app_theme) in [("dark", AppTheme::Dark), ("light", AppTheme::Light)] {
        for (layout, width) in [("wide", 1360.0), ("narrow", 760.0)] {
            let name = format!("{layout}-{theme_name}");
            let mut rig = Rig::new(&format!("ext-gallery-{name}"));
            rig.manager.save_open();
            let mut screen = Screen::new(egui::vec2(width, 860.0));
            theme::apply(&screen.ctx, app_theme);

            // A clean editor follows the file, and says so.
            outside_edit(&mut rig, OUTSIDE, Instant::now() + Duration::from_secs(10));
            let output = frames(&mut rig, &mut screen, 6);
            screen.save(&format!("outside-reloaded-{name}"), output);

            // Unsaved typing: the file changes again and the question appears.
            let typed = format!(
                "{}// my own change\n",
                rig.manager.open.as_ref().unwrap().editor.source
            );
            rig.set_source(&typed);
            let changed = OUTSIDE.replace("20", "50");
            outside_edit(&mut rig, &changed, Instant::now() + Duration::from_secs(20));
            let output = frames(&mut rig, &mut screen, 6);
            screen.save(&format!("outside-question-{name}"), output);
        }
    }
}
