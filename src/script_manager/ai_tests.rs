//! The assistant through the whole Script Manager window: the panel opening and drawing, a
//! scripted reply becoming a proposal, accepting it, and the cases that must not change the
//! editor. A scripted provider stands in for the network.

use std::sync::Arc;
use std::time::Duration;

use super::ai_panel::PanelAction;
use super::debug_tests::Rig;
use super::*;
use crate::service::llm::mock::MockProvider;
use crate::service::llm::{ChatMessage, ChatRequest, LlmError, ProviderKind};
use eframe::egui;

fn request() -> ChatRequest {
    ChatRequest {
        model: "m".to_owned(),
        system: "s".to_owned(),
        messages: vec![ChatMessage::user("hi")],
        max_tokens: 10,
    }
}

fn source(rig: &Rig) -> String {
    rig.manager.open.as_ref().unwrap().editor.source.clone()
}

/// Sends a request that the scripted provider answers with `reply`, and runs frames until it ends.
fn ask(rig: &mut Rig, reply: Result<Vec<String>, LlmError>) {
    let base = source(rig);
    rig.manager.ai.start(
        Box::new(MockProvider::replying(vec![reply])),
        request(),
        "make a change".to_owned(),
        base,
        Arc::new(|| {}),
    );
    for _ in 0..400 {
        rig.frame();
        if !rig.manager.ai.is_running() {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("the reply never finished");
}

fn script_reply(script: &str) -> Result<Vec<String>, LlmError> {
    MockProvider::text(&["Changed it.\n```pine\n", script, "\n```\n"])
}

fn open_panel(rig: &mut Rig) {
    rig.manager.cmds.push(Cmd::ToggleAiPanel);
    rig.frame();
    assert!(rig.manager.prefs.ai_open);
}

#[test]
fn the_panel_opens_closes_and_remembers_it() {
    let mut rig = Rig::new("ai-open");
    assert!(!rig.manager.prefs.ai_open);
    open_panel(&mut rig);
    rig.manager.ai_action(PanelAction::Close, &rig.ctx.clone());
    assert!(!rig.manager.prefs.ai_open);
}

#[test]
fn the_panel_draws_in_every_state_at_a_small_size() {
    let mut rig = Rig::new("ai-draw");
    rig.screen = Some(egui::Rect::from_min_size(
        egui::Pos2::ZERO,
        egui::vec2(700.0, 420.0),
    ));
    open_panel(&mut rig);
    // Empty, with the key missing.
    rig.frame();
    // A conversation with a proposal, a notice and a long diff.
    let base = source(&rig);
    let long: String = (0..120).map(|i| format!("v{i} = {i}\n")).collect();
    ask(&mut rig, script_reply(&format!("{base}{long}")));
    assert!(rig.manager.ai.proposal.is_some());
    rig.manager.ai.notice = Some(ai::Notice::error("Something failed."));
    rig.frame();
    rig.frame();
}

#[test]
fn a_reply_becomes_a_proposal_that_changes_nothing_until_accepted() {
    let mut rig = Rig::new("ai-propose");
    open_panel(&mut rig);
    let before = source(&rig);
    let new = format!("{before}// by the assistant\n");
    ask(&mut rig, script_reply(&new));
    let proposal = rig.manager.ai.proposal.clone().expect("a proposal");
    assert_eq!(proposal.base, before);
    assert_eq!(proposal.script, new);
    assert_eq!(source(&rig), before, "nothing is applied yet");
}

#[test]
fn accepting_puts_the_script_in_the_editor_and_clears_the_proposal() {
    let mut rig = Rig::new("ai-accept");
    open_panel(&mut rig);
    let before = source(&rig);
    let new = format!("{before}// by the assistant\n");
    ask(&mut rig, script_reply(&new));
    rig.manager.cmds.push(Cmd::Ai(PanelAction::Accept));
    rig.frame();
    assert_eq!(source(&rig), new);
    assert!(rig.manager.ai.proposal.is_none());
    assert!(!rig.manager.ai.notice.as_ref().unwrap().error);
    assert!(rig.manager.open.as_ref().unwrap().editor.is_dirty());
}

#[test]
fn accepting_is_refused_when_the_editor_changed_since() {
    let mut rig = Rig::new("ai-stale");
    open_panel(&mut rig);
    let before = source(&rig);
    ask(
        &mut rig,
        script_reply(&format!("{before}// by the assistant\n")),
    );
    let typed = format!("{before}// typed meanwhile\n");
    rig.set_source(&typed);
    rig.manager.cmds.push(Cmd::Ai(PanelAction::Accept));
    rig.frame();
    assert_eq!(source(&rig), typed, "the user's typing is untouched");
    assert!(rig.manager.ai.proposal.is_none());
    assert!(rig.manager.ai.notice.as_ref().unwrap().error);
}

#[test]
fn discard_drops_the_proposal_and_keeps_the_conversation() {
    let mut rig = Rig::new("ai-discard");
    open_panel(&mut rig);
    let before = source(&rig);
    ask(&mut rig, script_reply(&format!("{before}// x\n")));
    rig.manager.cmds.push(Cmd::Ai(PanelAction::Discard));
    rig.frame();
    assert!(rig.manager.ai.proposal.is_none());
    assert_eq!(rig.manager.ai.turns.len(), 2);
    assert_eq!(source(&rig), before);
}

#[test]
fn opening_another_script_starts_a_new_conversation() {
    let mut rig = Rig::new("ai-switch");
    open_panel(&mut rig);
    let before = source(&rig);
    ask(&mut rig, script_reply(&format!("{before}// x\n")));
    assert!(rig.manager.ai.proposal.is_some());
    let other = rig.manager.scripts[1].id.clone();
    rig.manager.open_script(&other);
    rig.frame();
    assert!(rig.manager.ai.proposal.is_none());
    assert!(rig.manager.ai.turns.is_empty());
    assert_eq!(rig.manager.ai.script_id.as_deref(), Some(other.as_str()));
}

#[test]
fn start_over_clears_the_conversation() {
    let mut rig = Rig::new("ai-clear");
    open_panel(&mut rig);
    let before = source(&rig);
    ask(&mut rig, script_reply(&format!("{before}// x\n")));
    rig.manager.cmds.push(Cmd::Ai(PanelAction::Clear));
    rig.frame();
    assert!(rig.manager.ai.turns.is_empty() && rig.manager.ai.proposal.is_none());
}

#[test]
fn a_failed_reply_leaves_the_editor_alone_and_says_why() {
    let mut rig = Rig::new("ai-fail");
    open_panel(&mut rig);
    let before = source(&rig);
    ask(&mut rig, Err(LlmError::Auth("bad key".to_owned())));
    assert_eq!(source(&rig), before);
    let notice = rig.manager.ai.notice.clone().unwrap();
    assert!(notice.error && notice.text.contains("bad key"));
}

#[test]
fn the_panel_says_what_is_missing_before_a_request_can_be_made() {
    let mut rig = Rig::new("ai-setup");
    // The test key store is empty and no environment variable is set.
    let setup = rig.manager.ai_setup();
    assert_eq!(
        setup.problem.as_deref(),
        Some("Add your OpenAI API key to start")
    );

    rig.manager.ai.prefs.provider = ProviderKind::Compatible;
    assert!(rig.manager.ai_setup().problem.unwrap().contains("address"));
    rig.manager.ai.prefs.compatible_base_url = "http://localhost:11434/v1".to_owned();
    assert!(rig.manager.ai_setup().problem.unwrap().contains("model"));
    rig.manager
        .ai
        .prefs
        .set_model(ProviderKind::Compatible, "llama3");
    assert_eq!(rig.manager.ai_setup().problem, None);
}

#[test]
fn sending_without_a_key_explains_instead_of_sending() {
    let mut rig = Rig::new("ai-nokey");
    open_panel(&mut rig);
    rig.manager.ai.input = "make it red".to_owned();
    rig.manager.cmds.push(Cmd::Ai(PanelAction::Send));
    rig.frame();
    assert!(!rig.manager.ai.is_running());
    let notice = rig.manager.ai.notice.clone().unwrap();
    assert!(
        notice.error && notice.text.contains("API key"),
        "{}",
        notice.text
    );
    assert_eq!(rig.manager.ai.input, "make it red", "the message is kept");
}

#[test]
fn open_settings_asks_the_app_for_the_ai_page() {
    let mut rig = Rig::new("ai-settings");
    let ctx = rig.ctx.clone();
    rig.manager.ai_action(PanelAction::OpenSettings, &ctx);
    assert!(
        rig.manager
            .actions
            .contains(&ScriptManagerAction::OpenAiSettings)
    );
}

#[test]
fn the_panel_state_is_saved_with_the_layout() {
    let mut rig = Rig::new("ai-prefs");
    open_panel(&mut rig);
    rig.frame();
    let saved = LayoutPrefs::load(&rig.shared);
    assert!(saved.ai_open);
}
