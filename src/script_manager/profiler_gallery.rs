//! Pictures of the profiler in the whole window, in both themes, for a person to look at. Run by
//! hand: `DEGENVIEW_GALLERY_DIR=… cargo test --lib script_manager::profiler_gallery -- --ignored`.

use std::time::Duration;

use eframe::egui;

use super::debug_tests::Rig;
use super::profiler::Status;
use super::*;
use crate::ui::render_tests::{Screen, gallery_dir};
use crate::ui::theme::AppTheme;

const SOURCE: &str = "//@version=6
indicator(\"Trend strength\", overlay = false)
length = input.int(20, \"Length\")

weight(i) =>
    math.pow(1.0 - i / 40.0, 2)

smooth = ta.sma(close, length)
atr = ta.atr(14)

score = 0.0
for i = 0 to 39
    w = weight(i)
    delta = (close[i] - smooth[i]) / atr
    score += w * delta

trend = 0
if score > 1.0
    trend := 1
else if score < -1.0
    trend := -1

plot(score, \"Score\")
plot(trend, \"Trend\")
";

struct Scene<'a> {
    rig: &'a mut Rig,
    screen: Screen,
    last: Option<egui::FullOutput>,
    name: String,
}

impl Scene<'_> {
    fn frames(&mut self, count: usize) {
        self.frames_with(count, Vec::new());
    }

    fn frames_with(&mut self, count: usize, mut events: Vec<egui::Event>) {
        for _ in 0..count {
            theme::sync(&self.screen.ctx);
            let manager = &mut self.rig.manager;
            self.last = Some(self.screen.run(std::mem::take(&mut events), |ui| {
                manager.ui(ui, None);
            }));
            std::thread::sleep(Duration::from_millis(15));
        }
    }

    fn until(&mut self, done: impl Fn(&Status) -> bool) {
        for _ in 0..600 {
            let status = self
                .rig
                .manager
                .profiler
                .status(self.rig.manager.profiler_now.as_ref());
            if done(&status) {
                return;
            }
            self.frames(1);
        }
        panic!("the profiler never got there");
    }

    fn save(&mut self, label: &str) {
        let output = self.last.take().expect("a frame");
        self.screen
            .save(&format!("profiler-{label}-{}", self.name), output);
    }
}

#[test]
#[ignore = "writes images for a person to look at"]
fn gallery_of_the_profiler() {
    if gallery_dir().is_none() {
        return;
    }
    for (theme_name, app_theme) in [("dark", AppTheme::Dark), ("light", AppTheme::Light)] {
        for (layout, width) in [("wide", 1400.0), ("narrow", 820.0)] {
            let mut rig = Rig::new(&format!("profiler-gallery-{layout}-{theme_name}"));
            rig.manager
                .preview
                .set_candles_for_test(crate::script_manager::debug_tests::bars(400));
            let screen = Screen::new(egui::vec2(width, 860.0));
            theme::apply(&screen.ctx, app_theme);
            let mut scene = Scene {
                rig: &mut rig,
                screen,
                last: None,
                name: format!("{layout}-{theme_name}"),
            };
            scene.rig.set_source(SOURCE);
            scene.frames(3);
            scene.rig.manager.prefs.drawer_tab = prefs::DrawerTab::Profile;
            scene.rig.manager.prefs.drawer_open = true;
            scene.frames(2);
            scene.save("off");

            scene.rig.manager.cmds.push(Cmd::ProfilerToggle);
            scene.until(|status| *status == Status::Ready);
            scene.frames(4);
            scene.save("ready");

            // The pointer over line 13's bar in the profile column: its figures, and the block
            // it sits in washed in the code.
            let (x, y) = if layout == "wide" {
                (850.0, 232.0)
            } else {
                (520.0, 233.0)
            };
            scene.frames_with(16, vec![egui::Event::PointerMoved(egui::pos2(x, y))]);
            scene.save("hover");

            scene.frames_with(
                2,
                vec![egui::Event::PointerMoved(egui::pos2(width - 20.0, 10.0))],
            );
            scene.rig.set_source(&SOURCE.replace("39", "20"));
            scene.frames(4);
            scene.save("stale");

            scene.rig.set_source(SOURCE);
            scene.rig.manager.cmds.push(Cmd::ProfilerRun);
            scene.frames(1);
            scene.save("running-or-ready");
            scene.until(|status| *status == Status::Ready);
        }
    }
}
