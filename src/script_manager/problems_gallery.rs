//! Pictures of a script that compiles and fails when it runs, in both themes, for a person to look
//! at. Run by hand: `DEGENVIEW_GALLERY_DIR=… cargo test --lib script_manager::problems_gallery -- --ignored`.

use std::time::{Duration, Instant};

use eframe::egui;

use super::debug_tests::Rig;
use super::prefs::DrawerTab;
use super::*;
use crate::ui::render_tests::{Screen, gallery_dir};
use crate::ui::theme::AppTheme;

const FAILS: &str = "//@version=6
indicator(\"Volume Delta Map\")
length = input.int(20, \"Length\")
var values = array.new_float()
if barstate.islast
    cells = length * 4
    if values.size() != cells
        array.push(values, 1.0)
    array.get(values, cells)
plot(ta.sma(close, length))
";

/// A typo on a line that never runs: the compiler rejects it, so nothing runs.
const TYPO: &str = "//@version=6
indicator(\"Typo\")
var values = array.new_float()
if close < 0
    array.pusj(values, close)
plot(close)
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

fn until(rig: &mut Rig, screen: &mut Screen, what: &str, done: impl Fn(&Rig) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !done(rig) {
        frames(rig, screen, 1);
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
    }
}

#[test]
#[ignore = "writes images for a person to look at"]
fn gallery_of_a_runtime_failure() {
    if gallery_dir().is_none() {
        return;
    }
    for (name, app_theme) in [("dark", AppTheme::Dark), ("light", AppTheme::Light)] {
        let mut rig = Rig::new(&format!("problems-gallery-{name}"));
        let mut screen = Screen::new(egui::vec2(1360.0, 860.0));
        theme::apply(&screen.ctx, app_theme);
        rig.set_source(FAILS);
        until(&mut rig, &mut screen, "the failure", |rig| {
            rig.manager.preview.runtime_error().is_some()
        });
        rig.manager.prefs.drawer_open = true;
        rig.manager.prefs.drawer_tab = DrawerTab::Problems;
        let output = frames(&mut rig, &mut screen, 4);
        screen.save(&format!("problems-{name}"), output);

        // The profile of the same script: it fails the same way, and says so in its own tab.
        rig.manager.cmds.push(Cmd::ProfilerToggle);
        until(&mut rig, &mut screen, "the profile", |rig| {
            rig.manager
                .profiler
                .failure(rig.manager.profiler_now.as_ref())
                .is_some()
        });
        rig.manager.prefs.drawer_tab = DrawerTab::Profile;
        let output = frames(&mut rig, &mut screen, 4);
        screen.save(&format!("profile-{name}"), output);

        // A compile error with a hint, before anything runs.
        rig.set_source(TYPO);
        rig.manager.prefs.drawer_tab = DrawerTab::Problems;
        until(&mut rig, &mut screen, "the error", |rig| {
            rig.manager
                .open
                .as_ref()
                .is_some_and(|open| !open.editor.diagnostics.is_empty())
        });
        let output = frames(&mut rig, &mut screen, 6);
        screen.save(&format!("typo-{name}"), output);
    }
}
