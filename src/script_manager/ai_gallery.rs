//! Pictures of the assistant's panel in the whole window, in both themes, for a person to look at.
//! Run by hand: `DEGENVIEW_GALLERY_DIR=… cargo test --lib script_manager::ai_gallery -- --ignored`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use eframe::egui;

use super::ai_panel::PanelAction;
use super::ai_prompt::Mode;
use super::debug_tests::Rig;
use super::*;
use crate::service::keychain_ai_keys::AiKeyStore;
use crate::service::llm::{ChatMessage, ChatRequest, Done, LlmError, LlmProvider, ProviderKind};
use crate::ui::render_tests::{Screen, gallery_dir};
use crate::ui::theme::AppTheme;

const PLAN_REPLY: &str = "The length is hard-coded in two places, so one input is the cleanest fix.\n\n## Plan\n1. Declare `length = input.int(14, \"Length\", minval = 2)` near the top.\n2. Replace the literal `14` in `ta.rsi(close, 14)` with **length**.\n3. Show it in the title so the chart reflects the setting.\n\nThe core of it is small:\n\n```pine\nlength = input.int(14, \"Length\", minval = 2)\nvalue = ta.rsi(close, length)\n```\n\n> Inputs appear in the drawer's Inputs tab with no extra work.\n\nSee [the Pine docs](https://www.tradingview.com/pine-script-docs/) for `input.int`.";

/// Streams the start of a reply and then waits, so the "writing" state can be photographed.
struct Stalled;

impl LlmProvider for Stalled {
    fn stream_chat(
        &self,
        _: &ChatRequest,
        cancel: &AtomicBool,
        on_text: &mut dyn FnMut(&str),
    ) -> Result<Done, LlmError> {
        on_text(
            "Adding a length input and using it in the average.\n```pine\n//@version=6\nindicator(\"RSI Zones\")\nlength = input.int(14)\n",
        );
        while !cancel.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(10));
        }
        Err(LlmError::Cancelled)
    }

    fn list_models(&self) -> Result<Vec<String>, LlmError> {
        Ok(Vec::new())
    }
}

struct Scene<'a> {
    rig: &'a mut Rig,
    screen: Screen,
    last: Option<egui::FullOutput>,
    name: String,
}

impl Scene<'_> {
    fn frames(&mut self, count: usize) {
        for _ in 0..count {
            theme::sync(&self.screen.ctx);
            let manager = &mut self.rig.manager;
            self.last = Some(self.screen.run(Vec::new(), |ui| {
                manager.ui(ui, None);
            }));
            std::thread::sleep(Duration::from_millis(15));
        }
    }

    fn save(&mut self, label: &str) {
        let output = self.last.take().expect("a frame");
        self.screen
            .save(&format!("assistant-{label}-{}", self.name), output);
    }
}

#[test]
#[ignore = "writes images for a person to look at"]
fn gallery_of_the_assistant() {
    if gallery_dir().is_none() {
        return;
    }
    for (theme_name, app_theme) in [("dark", AppTheme::Dark), ("light", AppTheme::Light)] {
        for (layout, width) in [("wide", 1360.0), ("narrow", 760.0)] {
            let mut rig = Rig::new(&format!("ai-gallery-{layout}-{theme_name}"));
            rig.manager.prefs.ai_open = true;
            rig.manager.prefs.debug_open = layout == "wide" && theme_name == "dark";
            let screen = Screen::new(egui::vec2(width, 860.0));
            theme::apply(&screen.ctx, app_theme);
            let mut scene = Scene {
                rig: &mut rig,
                screen,
                last: None,
                name: format!("{layout}-{theme_name}"),
            };

            // No key yet: the panel says so and offers Settings.
            AiKeyStore::shared().remove(ProviderKind::OpenAi);
            scene.frames(3);
            scene.save("no-key");

            // Ready, nothing asked yet.
            AiKeyStore::shared()
                .save(ProviderKind::OpenAi, "sk-gallery")
                .unwrap();
            scene.frames(3);
            scene.save("empty");

            // A Plan reply: markdown, a snippet, a quote, a link, and the Implement button.
            let base = scene
                .rig
                .manager
                .open
                .as_ref()
                .unwrap()
                .editor
                .source
                .clone();
            scene.rig.manager.ai.start(
                Box::new(crate::service::llm::mock::MockProvider::replying(vec![
                    crate::service::llm::mock::MockProvider::text(&[PLAN_REPLY]),
                ])),
                ChatRequest {
                    model: "m".into(),
                    system: String::new(),
                    messages: vec![ChatMessage::user("x")],
                    max_tokens: 10,
                },
                "How would I add a length input?".to_owned(),
                base,
                Mode::Plan,
                Arc::new(|| {}),
            );
            scene.frames(12);
            scene.save("plan");
            scene.rig.manager.ai.clear();

            // Edit mode from here on.
            scene.rig.manager.ai.set_mode(Mode::Edit);
            scene.frames(3);
            scene.save("empty-edit");

            // Writing.
            let base = scene
                .rig
                .manager
                .open
                .as_ref()
                .unwrap()
                .editor
                .source
                .clone();
            scene.rig.manager.ai.start(
                Box::new(Stalled),
                ChatRequest {
                    model: "m".into(),
                    system: String::new(),
                    messages: vec![ChatMessage::user("x")],
                    max_tokens: 10,
                },
                "Add an input for the length".to_owned(),
                base.clone(),
                Mode::Edit,
                Arc::new(|| {}),
            );
            scene.frames(12);
            scene.save("writing");
            scene.rig.manager.ai.stop();

            // A proposal with several kinds of change over a longer script.
            let rsi = super::demo::BUNDLED
                .iter()
                .find(|demo| demo.name == "RSI Zones")
                .unwrap()
                .source;
            let padding: String = (1..=22).map(|n| format!("// note {n}\n")).collect();
            let base = format!("{rsi}\n{padding}plot(close)\n");
            scene.rig.set_source(&base);
            let changed = base
                .replace("input.int(14, \"Length\"", "input.int(21, \"Length\"")
                .replace(
                    "value = ta.rsi(source, length)\n",
                    "value = ta.rsi(source, length)\nsmooth = ta.ema(value, 3)\n",
                )
                .replace(
                    "color = color.purple, linewidth = 2",
                    "color = color.orange, linewidth = 3",
                )
                .replace("hline(50, \"Midline\", color = color.gray)\n", "")
                .replace(
                    "plot(close)\n",
                    "plot(close)\nalertcondition(value > overbought, \"Overbought\")\n",
                );
            let mock = crate::service::llm::mock::MockProvider::replying(vec![
                crate::service::llm::mock::MockProvider::text(&[&format!(
                    "I made the length 21, added a smoothed line, restyled the plot, dropped the midline and added an alert.\n```pine\n{changed}```\n"
                )]),
            ]);
            scene.rig.manager.ai.start(
                Box::new(mock),
                ChatRequest {
                    model: "m".into(),
                    system: String::new(),
                    messages: vec![ChatMessage::user("x")],
                    max_tokens: 10,
                },
                "Use 21, smooth it, restyle the plot, drop the midline, add an alert".to_owned(),
                base,
                Mode::Edit,
                Arc::new(|| {}),
            );
            scene.frames(12);
            scene.save("proposal");

            // The review sheet: everything accepted, then one rejected and another selected.
            scene.rig.manager.cmds.push(Cmd::Ai(PanelAction::Review));
            scene.frames(6);
            scene.save("review");
            let ctx = scene.screen.ctx.clone();
            scene
                .rig
                .manager
                .ai_review_action(super::ai_review_view::ReviewAction::Toggle(2), &ctx);
            scene
                .rig
                .manager
                .ai_review_action(super::ai_review_view::ReviewAction::Select(3), &ctx);
            scene.frames(6);
            scene.save("review-rejected");
            scene
                .rig
                .manager
                .ai_review_action(super::ai_review_view::ReviewAction::Close, &ctx);
            scene.frames(3);

            // The result of Apply.
            scene.rig.manager.cmds.push(Cmd::Ai(PanelAction::Accept));
            scene.frames(4);
            scene.save("applied");
        }
    }
}
