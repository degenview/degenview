//! The assistant through the whole Script Manager window: the panel opening and drawing, a
//! scripted reply becoming a proposal, accepting it, and the cases that must not change the
//! editor. A scripted provider stands in for the network.

use std::sync::Arc;
use std::time::Duration;

use super::ai_agent::AgentSetup;
use super::ai_history::History;
use super::ai_host::diagnostics_of;
use super::ai_models::ModelList;
use super::ai_panel::PanelAction;
use super::ai_prompt::Mode;
use super::ai_review_view::ReviewAction;
use super::ai_tools::ToolBox;
use super::debug_tests::Rig;
use super::*;
use crate::service::llm::mock::{MockProvider, Scripted};
use crate::service::llm::prefs::AiPrefs;
use crate::service::llm::{ChatMessage, ChatRequest, LlmError, ProviderKind};
use eframe::egui;

fn request() -> ChatRequest {
    ChatRequest {
        model: "m".to_owned(),
        system: "s".to_owned(),
        messages: vec![ChatMessage::user("hi")],
        max_tokens: 10,
        tools: Vec::new(),
    }
}

fn source(rig: &Rig) -> String {
    rig.manager.open.as_ref().unwrap().editor.source.clone()
}

/// Sends an Edit request that the scripted provider answers with `reply`, and runs frames until it
/// ends.
fn ask(rig: &mut Rig, reply: Result<Vec<String>, LlmError>) {
    ask_in(rig, Mode::Edit, reply);
}

fn ask_in(rig: &mut Rig, mode: Mode, reply: Result<Vec<String>, LlmError>) {
    // As a user would: choose the mode, then send.
    assert!(rig.manager.ai.set_mode(mode));
    let base = source(rig);
    rig.manager.ai.start(
        Box::new(MockProvider::replying(vec![reply])),
        request(),
        "make a change".to_owned(),
        base,
        mode,
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

/// Sends a request with tools, answered by `turns`, and runs frames until it ends.
fn ask_with_tools(rig: &mut Rig, mode: Mode, turns: Vec<Result<Scripted, LlmError>>) {
    begin_with_tools(rig, mode, turns);
    wait_for_request(rig);
}

/// Sends a request with tools without waiting for it.
fn begin_with_tools(rig: &mut Rig, mode: Mode, turns: Vec<Result<Scripted, LlmError>>) {
    assert!(rig.manager.ai.set_mode(mode));
    let base = source(rig);
    let feed = rig.manager.preview.debug_feed(model::now_ms(), false);
    let tools = ToolBox::new(mode, &base, diagnostics_of).with_bars(feed);
    let request = ChatRequest {
        tools: tools.specs(),
        ..request()
    };
    rig.manager.ai.start_agent(
        Box::new(MockProvider::turns(turns)),
        request,
        Some(AgentSetup {
            tools,
            fallback_system: String::new(),
        }),
        "make a change".to_owned(),
        base,
        mode,
        Arc::new(|| {}),
    );
}

fn wait_for_request(rig: &mut Rig) {
    for _ in 0..400 {
        rig.frame();
        if !rig.manager.ai.is_running() {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("the request never finished");
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
    ask(&mut rig, Err(LlmError::auth("bad key")));
    assert_eq!(source(&rig), before);
    let notice = rig.manager.ai.notice.clone().unwrap();
    assert!(notice.error && notice.error_report().detail.contains("bad key"));
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

#[test]
fn the_panel_opens_in_ask_mode() {
    let mut rig = Rig::new("ai-plan-default");
    open_panel(&mut rig);
    assert_eq!(rig.manager.ai.mode, Mode::Ask);
}

#[test]
fn a_question_in_ask_mode_leaves_the_editor_and_the_proposal_alone() {
    let mut rig = Rig::new("ai-plan-asks");
    open_panel(&mut rig);
    let before = source(&rig);
    // Even a model that answers with a whole new script changes nothing.
    ask_in(
        &mut rig,
        Mode::Ask,
        script_reply(&format!("{before}// sneaky\n")),
    );
    assert!(rig.manager.ai.proposal.is_none());
    assert_eq!(source(&rig), before);
    // And Apply, were it ever sent, does nothing in Ask.
    rig.manager.cmds.push(Cmd::Ai(PanelAction::Accept));
    rig.frame();
    assert_eq!(source(&rig), before);
}

#[test]
fn accept_is_refused_in_ask_even_if_a_proposal_somehow_exists() {
    let mut rig = Rig::new("ai-plan-accept");
    open_panel(&mut rig);
    let before = source(&rig);
    ask(&mut rig, script_reply(&format!("{before}// x\n")));
    assert!(rig.manager.ai.proposal.is_some());
    rig.manager.ai.mode = Mode::Ask;
    rig.manager.cmds.push(Cmd::Ai(PanelAction::Accept));
    rig.frame();
    assert_eq!(source(&rig), before, "Ask mode applies nothing");
}

#[test]
fn the_mode_switch_works_and_is_locked_while_a_change_waits() {
    let mut rig = Rig::new("ai-mode-switch");
    open_panel(&mut rig);
    rig.manager
        .cmds
        .push(Cmd::Ai(PanelAction::SetMode(Mode::Edit)));
    rig.frame();
    assert_eq!(rig.manager.ai.mode, Mode::Edit);

    let before = source(&rig);
    ask(&mut rig, script_reply(&format!("{before}// x\n")));
    rig.manager
        .cmds
        .push(Cmd::Ai(PanelAction::SetMode(Mode::Ask)));
    rig.frame();
    assert_eq!(
        rig.manager.ai.mode,
        Mode::Edit,
        "locked while a change waits"
    );

    rig.manager.cmds.push(Cmd::Ai(PanelAction::Discard));
    rig.frame();
    rig.manager
        .cmds
        .push(Cmd::Ai(PanelAction::SetMode(Mode::Ask)));
    rig.frame();
    assert_eq!(rig.manager.ai.mode, Mode::Ask);
}

#[test]
fn implement_switches_to_edit_and_asks_for_the_plan() {
    let mut rig = Rig::new("ai-implement");
    open_panel(&mut rig);
    ask_in(
        &mut rig,
        Mode::Ask,
        MockProvider::text(&["Plan:\n1. Add an input\n2. Use it\n"]),
    );
    assert!(rig.manager.ai.can_implement());
    rig.manager.cmds.push(Cmd::Ai(PanelAction::Implement));
    rig.frame();
    // The test key store is empty, so the request itself is refused; what matters is that it was
    // made as an Edit request carrying the implement message.
    assert_eq!(rig.manager.ai.mode, Mode::Edit);
    assert_eq!(rig.manager.ai.input, super::ai_prompt::IMPLEMENT_MESSAGE);
    assert!(rig.manager.ai.notice.as_ref().unwrap().error);
}

#[test]
fn implement_does_nothing_without_a_plan() {
    let mut rig = Rig::new("ai-implement-none");
    open_panel(&mut rig);
    rig.manager.cmds.push(Cmd::Ai(PanelAction::Implement));
    rig.frame();
    assert_eq!(rig.manager.ai.mode, Mode::Ask);
    assert!(rig.manager.ai.input.is_empty());
}

#[test]
fn the_panel_draws_a_plan_reply_with_code_and_the_implement_button() {
    let mut rig = Rig::new("ai-plan-draw");
    rig.screen = Some(egui::Rect::from_min_size(
        egui::Pos2::ZERO,
        egui::vec2(700.0, 460.0),
    ));
    open_panel(&mut rig);
    ask_in(
        &mut rig,
        Mode::Ask,
        MockProvider::text(&[
            "## Plan\nThe **length** is hard-coded. See [docs](https://example.com).\n",
            "1. Add `input.int`\n2. Use it:\n```pine\nlength = input.int(14)\n```\n- done *soon*\n",
        ]),
    );
    rig.frame();
    rig.frame();
    assert!(rig.manager.ai.can_implement());
}

/// A script of 33 lines, and the same with three changes far enough apart to be three hunks.
fn base_and_three_changes() -> (String, String) {
    let mut lines: Vec<String> = vec!["//@version=6".into(), "indicator(\"t\")".into()];
    lines.extend((1..=30).map(|i| format!("v{i} = {i}")));
    lines.push("plot(v1)".into());
    let base = lines.join("\n") + "\n";
    let new = base
        .replace("v3 = 3\n", "v3 = 33\n")
        .replace("v15 = 15\n", "v15 = 1500\n")
        .replace("v28 = 28\n", "v28 = 2800\n");
    (base, new)
}

/// A rig with the assistant panel open and `new` proposed over `base`.
fn proposing(tag: &str, base: &str, new: &str) -> Rig {
    let mut rig = Rig::new(tag);
    open_panel(&mut rig);
    rig.set_source(base);
    ask(&mut rig, script_reply(new));
    assert_eq!(
        rig.manager
            .ai
            .proposal
            .as_ref()
            .unwrap()
            .changes
            .hunks()
            .len(),
        3
    );
    rig
}

fn review(rig: &mut Rig, action: ReviewAction) {
    let ctx = rig.ctx.clone();
    rig.manager.ai_review_action(action, &ctx);
}

fn open_review(rig: &mut Rig) {
    rig.manager.cmds.push(Cmd::Ai(PanelAction::Review));
    rig.frame();
    assert!(rig.manager.ai.review.is_some());
}

#[test]
fn the_review_sheet_opens_over_a_proposal_and_draws_at_both_sizes() {
    let (base, new) = base_and_three_changes();
    let mut rig = proposing("ai-review-open", &base, &new);
    for size in [egui::vec2(1400.0, 900.0), egui::vec2(700.0, 420.0)] {
        rig.screen = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size));
        open_review(&mut rig);
        rig.frame();
        rig.frame();
        let review = rig.manager.ai.review.as_ref().unwrap();
        assert_eq!(review.accepted, vec![true; 3]);
        assert!(!review.rows.is_empty());
        rig.manager.ai.review = None;
    }
    assert_eq!(source(&rig), base, "looking changes nothing");
}

#[test]
fn applying_a_subset_applies_only_the_accepted_changes() {
    let (base, new) = base_and_three_changes();
    let mut rig = proposing("ai-review-subset", &base, &new);
    open_review(&mut rig);
    review(&mut rig, ReviewAction::Toggle(1));
    review(&mut rig, ReviewAction::Apply);
    let expected = base
        .replace("v3 = 3\n", "v3 = 33\n")
        .replace("v28 = 28\n", "v28 = 2800\n");
    assert_eq!(
        source(&rig),
        expected,
        "the rejected middle change is not applied"
    );
    assert!(rig.manager.ai.proposal.is_none() && rig.manager.ai.review.is_none());
    let notice = rig.manager.ai.notice.clone().unwrap();
    assert!(
        !notice.error && notice.text.contains("2 of 3"),
        "{}",
        notice.text
    );
}

#[test]
fn accepting_everything_in_the_sheet_equals_apply_all() {
    let (base, new) = base_and_three_changes();
    let mut rig = proposing("ai-review-all", &base, &new);
    open_review(&mut rig);
    review(&mut rig, ReviewAction::Apply);
    assert_eq!(source(&rig), new);
    assert!(
        rig.manager
            .ai
            .notice
            .as_ref()
            .unwrap()
            .text
            .contains("all changes")
    );
}

#[test]
fn a_partly_applied_review_is_one_undo_step() {
    let (base, new) = base_and_three_changes();
    let mut rig = proposing("ai-review-undo", &base, &new);
    open_review(&mut rig);
    review(&mut rig, ReviewAction::Toggle(0));
    review(&mut rig, ReviewAction::Apply);
    let applied = source(&rig);
    assert_ne!(applied, base);
    rig.frame();
    rig.frame();
    rig.frame_with(vec![egui::Event::Key {
        key: egui::Key::Z,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::COMMAND,
    }]);
    assert_eq!(source(&rig), base, "one undo brings back the whole base");
}

#[test]
fn rejecting_everything_leaves_apply_with_nothing_to_do() {
    let (base, new) = base_and_three_changes();
    let mut rig = proposing("ai-review-none", &base, &new);
    open_review(&mut rig);
    review(&mut rig, ReviewAction::SetAll(false));
    review(&mut rig, ReviewAction::Apply);
    assert_eq!(source(&rig), base);
    assert!(rig.manager.ai.review.is_some(), "the sheet stays open");
    assert!(rig.manager.ai.proposal.is_some());
}

#[test]
fn the_sheet_refuses_to_apply_over_text_typed_since() {
    let (base, new) = base_and_three_changes();
    let mut rig = proposing("ai-review-stale", &base, &new);
    open_review(&mut rig);
    let typed = format!("{base}// typed meanwhile\n");
    rig.set_source(&typed);
    review(&mut rig, ReviewAction::Apply);
    assert_eq!(source(&rig), typed);
    assert!(rig.manager.ai.proposal.is_none() && rig.manager.ai.review.is_none());
    assert!(rig.manager.ai.notice.as_ref().unwrap().error);
}

#[test]
fn escape_closes_the_sheet_and_keeps_the_proposal() {
    let (base, new) = base_and_three_changes();
    let mut rig = proposing("ai-review-esc", &base, &new);
    open_review(&mut rig);
    rig.press(egui::Key::Escape);
    assert!(rig.manager.ai.review.is_none());
    assert!(rig.manager.ai.proposal.is_some());
    assert_eq!(source(&rig), base);
}

#[test]
fn discard_all_in_the_sheet_drops_the_proposal() {
    let (base, new) = base_and_three_changes();
    let mut rig = proposing("ai-review-discard", &base, &new);
    open_review(&mut rig);
    review(&mut rig, ReviewAction::DiscardAll);
    assert!(rig.manager.ai.proposal.is_none() && rig.manager.ai.review.is_none());
    assert_eq!(source(&rig), base);
}

#[test]
fn keys_move_between_changes_and_toggle_without_typing_into_the_editor() {
    let (base, new) = base_and_three_changes();
    let mut rig = proposing("ai-review-keys", &base, &new);
    open_review(&mut rig);
    rig.press(egui::Key::J);
    assert_eq!(rig.manager.ai.review.as_ref().unwrap().current, 1);
    rig.press(egui::Key::ArrowDown);
    assert_eq!(rig.manager.ai.review.as_ref().unwrap().current, 2);
    rig.press(egui::Key::K);
    assert_eq!(rig.manager.ai.review.as_ref().unwrap().current, 1);
    rig.press(egui::Key::Space);
    assert_eq!(
        rig.manager.ai.review.as_ref().unwrap().accepted,
        vec![true, false, true]
    );
    assert_eq!(source(&rig), base, "no key reached the editor");
}

#[test]
fn the_footer_recounts_compile_errors_for_the_selection() {
    let (base, new) = base_and_three_changes();
    // The middle change breaks the script; the others are harmless.
    let new = new.replace("v15 = 1500\n", "v15 = (1500\n");
    let mut rig = proposing("ai-review-recount", &base, &new);
    open_review(&mut rig);
    assert!(rig.manager.ai.review.as_ref().unwrap().errors > 0);
    review(&mut rig, ReviewAction::Toggle(1));
    assert_eq!(rig.manager.ai.review.as_ref().unwrap().errors, 0);
    review(&mut rig, ReviewAction::Toggle(1));
    assert!(rig.manager.ai.review.as_ref().unwrap().errors > 0);
}

#[test]
fn opening_a_fold_adds_the_hidden_lines() {
    let (base, new) = base_and_three_changes();
    let mut rig = proposing("ai-review-fold", &base, &new);
    open_review(&mut rig);
    let (before, key) = {
        let review = rig.manager.ai.review.as_ref().unwrap();
        let key = review
            .rows
            .iter()
            .find_map(|row| match row {
                super::ai_review::Row::Collapsed { key, .. } => Some(*key),
                _ => None,
            })
            .expect("a long script folds");
        (review.rows.len(), key)
    };
    review(&mut rig, ReviewAction::Expand(key));
    assert!(rig.manager.ai.review.as_ref().unwrap().rows.len() > before);
}

#[test]
fn the_sheet_does_not_open_in_ask_mode_or_without_a_proposal() {
    let (base, new) = base_and_three_changes();
    let mut rig = proposing("ai-review-plan", &base, &new);
    rig.manager.ai.mode = Mode::Ask;
    rig.manager.cmds.push(Cmd::Ai(PanelAction::Review));
    rig.frame();
    assert!(rig.manager.ai.review.is_none());

    let mut bare = Rig::new("ai-review-bare");
    open_panel(&mut bare);
    bare.manager.cmds.push(Cmd::Ai(PanelAction::Review));
    bare.frame();
    assert!(bare.manager.ai.review.is_none());
}

#[test]
fn a_new_request_or_start_over_closes_the_sheet() {
    let (base, new) = base_and_three_changes();
    let mut rig = proposing("ai-review-close", &base, &new);
    open_review(&mut rig);
    rig.manager.ai.clear();
    assert!(rig.manager.ai.review.is_none() && rig.manager.ai.proposal.is_none());
}

fn saved_turns(rig: &Rig) -> Vec<super::ai_prompt::Turn> {
    let id = rig.manager.ai.script_id.clone().expect("a script is open");
    History::load(&rig.shared.lock().unwrap()).get(&id)
}

#[test]
fn a_finished_reply_is_saved_and_comes_back_for_that_script_only() {
    let mut rig = Rig::new("ai-history");
    open_panel(&mut rig);
    ask_in(
        &mut rig,
        Mode::Ask,
        MockProvider::text(&["Because it is a moving average."]),
    );
    let saved = saved_turns(&rig);
    assert_eq!(saved.len(), 2);
    assert_eq!(saved, rig.manager.ai.turns);

    // Another script has its own, empty conversation; coming back restores this one.
    let first = rig.manager.ai.script_id.clone().unwrap();
    let other = rig.manager.scripts[1].id.clone();
    rig.manager.open_script(&other);
    rig.frame();
    assert!(rig.manager.ai.turns.is_empty());
    assert_eq!(rig.manager.ai.mode, Mode::Ask);
    rig.manager.open_script(&first);
    rig.frame();
    assert_eq!(rig.manager.ai.turns, saved);
}

#[test]
fn a_restarted_app_still_has_the_conversation() {
    let mut rig = Rig::new("ai-history-restart");
    open_panel(&mut rig);
    ask_in(&mut rig, Mode::Ask, MockProvider::text(&["An answer."]));
    let before = rig.manager.ai.turns.clone();
    let rig = rig.restarted();
    assert_eq!(rig.manager.ai.turns, before);
}

#[test]
fn start_over_deletes_the_saved_conversation() {
    let mut rig = Rig::new("ai-history-clear");
    open_panel(&mut rig);
    ask_in(&mut rig, Mode::Ask, MockProvider::text(&["An answer."]));
    assert!(!saved_turns(&rig).is_empty());
    rig.manager.cmds.push(Cmd::Ai(PanelAction::Clear));
    rig.frame();
    assert!(saved_turns(&rig).is_empty());
}

#[test]
fn deleting_a_script_deletes_its_conversation() {
    let mut rig = Rig::new("ai-history-delete");
    open_panel(&mut rig);
    ask_in(&mut rig, Mode::Ask, MockProvider::text(&["An answer."]));
    let id = rig.manager.ai.script_id.clone().unwrap();
    rig.manager.delete_script(&id);
    assert!(
        History::load(&rig.shared.lock().unwrap())
            .get(&id)
            .is_empty()
    );
}

#[test]
fn a_conversation_cleared_in_settings_is_not_brought_back() {
    let mut rig = Rig::new("ai-history-settings");
    open_panel(&mut rig);
    ask_in(&mut rig, Mode::Ask, MockProvider::text(&["An answer."]));
    // Settings clears the saved history while the conversation is on screen.
    {
        let store = rig.shared.lock().unwrap();
        let mut history = History::load(&store);
        history.clear();
        history.save(&store).unwrap();
    }
    let first = rig.manager.ai.script_id.clone().unwrap();
    let other = rig.manager.scripts[1].id.clone();
    rig.manager.open_script(&other);
    rig.frame();
    rig.manager.open_script(&first);
    rig.frame();
    assert!(rig.manager.ai.turns.is_empty(), "it stays cleared");
}

#[test]
fn edit_last_puts_the_message_back_and_forgets_the_exchange() {
    let mut rig = Rig::new("ai-edit-last");
    open_panel(&mut rig);
    ask_in(&mut rig, Mode::Ask, MockProvider::text(&["An answer."]));
    rig.manager.cmds.push(Cmd::Ai(PanelAction::EditLast));
    rig.frame();
    assert_eq!(rig.manager.ai.input, "make a change");
    assert!(rig.manager.ai.turns.is_empty());
    assert!(saved_turns(&rig).is_empty(), "the edit is saved too");
}

#[test]
fn retry_asks_the_last_message_again() {
    let mut rig = Rig::new("ai-retry");
    open_panel(&mut rig);
    ask_in(&mut rig, Mode::Ask, MockProvider::text(&["An answer."]));
    rig.manager.cmds.push(Cmd::Ai(PanelAction::Retry));
    rig.frame();
    // The test key store is empty, so the new request is refused; the exchange is gone and the
    // message is waiting to be sent again.
    assert!(rig.manager.ai.turns.is_empty());
    assert_eq!(rig.manager.ai.input, "make a change");
    assert!(rig.manager.ai.notice.as_ref().unwrap().error);
}

#[test]
fn retry_and_edit_do_nothing_without_an_exchange() {
    let mut rig = Rig::new("ai-retry-none");
    open_panel(&mut rig);
    for action in [PanelAction::Retry, PanelAction::EditLast] {
        rig.manager.cmds.push(Cmd::Ai(action));
        rig.frame();
    }
    assert!(rig.manager.ai.input.is_empty() && rig.manager.ai.notice.is_none());
}

#[test]
fn dismissing_a_notice_clears_it() {
    let mut rig = Rig::new("ai-dismiss");
    open_panel(&mut rig);
    rig.manager.ai.notice = Some(ai::Notice::error("Something."));
    rig.manager.cmds.push(Cmd::Ai(PanelAction::DismissNotice));
    rig.frame();
    assert!(rig.manager.ai.notice.is_none());
}

#[test]
fn choosing_a_model_or_provider_is_saved_where_settings_reads_it() {
    let mut rig = Rig::new("ai-model");
    open_panel(&mut rig);
    rig.manager
        .cmds
        .push(Cmd::Ai(PanelAction::SetModel("gpt-5-mini".into())));
    rig.frame();
    let saved = AiPrefs::load(&rig.shared.lock().unwrap());
    assert_eq!(saved.model(ProviderKind::OpenAi), "gpt-5-mini");
    assert_eq!(rig.manager.ai_setup().model, "gpt-5-mini");

    rig.manager
        .cmds
        .push(Cmd::Ai(PanelAction::SetProvider(ProviderKind::Anthropic)));
    rig.frame();
    let saved = AiPrefs::load(&rig.shared.lock().unwrap());
    assert_eq!(saved.provider, ProviderKind::Anthropic);
    assert_eq!(
        saved.model(ProviderKind::OpenAi),
        "gpt-5-mini",
        "the other model is kept"
    );
    assert_eq!(rig.manager.ai_setup().provider, "Anthropic");
}

#[test]
fn opening_the_model_menu_without_a_key_says_why_instead_of_loading() {
    let mut rig = Rig::new("ai-models-nokey");
    open_panel(&mut rig);
    rig.manager.cmds.push(Cmd::Ai(PanelAction::LoadModels));
    rig.frame();
    let list = rig.manager.ai.models.get(&ProviderKind::OpenAi).cloned();
    assert!(
        matches!(list, Some(ModelList::Failed(report)) if report.headline.contains("API key")),
        "no key: nothing to ask"
    );
}

#[test]
fn a_loaded_model_list_is_not_asked_for_again() {
    let mut rig = Rig::new("ai-models-cached");
    open_panel(&mut rig);
    rig.manager.ai.models.insert(
        ProviderKind::OpenAi,
        ModelList::Ready(vec!["gpt-5".to_owned()]),
    );
    rig.manager.cmds.push(Cmd::Ai(PanelAction::LoadModels));
    rig.frame();
    assert_eq!(
        rig.manager.ai.models.get(&ProviderKind::OpenAi),
        Some(&ModelList::Ready(vec!["gpt-5".to_owned()]))
    );
}

#[test]
fn the_setup_describes_what_a_request_sends_and_which_providers_are_usable() {
    let mut rig = Rig::new("ai-setup-context");
    let setup = rig.manager.ai_setup();
    assert!(setup.context.contains("lines"), "{}", setup.context);
    assert!(setup.context.contains("problem"), "{}", setup.context);
    assert_eq!(setup.providers.len(), 3);
    // With no keys only the keyless provider is ready.
    let ready: Vec<ProviderKind> = setup
        .providers
        .iter()
        .filter(|(_, ready)| *ready)
        .map(|(kind, _)| *kind)
        .collect();
    assert_eq!(ready, vec![ProviderKind::Compatible]);
    let _ = &mut rig;
}

#[test]
fn an_assistant_with_tools_edits_a_draft_and_the_editor_waits_for_apply() {
    let mut rig = Rig::new("ai-agent-edit");
    open_panel(&mut rig);
    let before = source(&rig);
    // The first line is the version comment, which a demo script has exactly once.
    let first = before.lines().next().unwrap().to_owned();
    assert_eq!(before.matches(&first).count(), 1);
    let turns = vec![
        Ok(Scripted::call(
            "t1",
            "read_script",
            serde_json::json!({"start": 1, "end": 3}),
        )),
        Ok(Scripted::call(
            "t2",
            "edit_script",
            serde_json::json!({"old": first, "new": format!("{first}\n// by the assistant")}),
        )),
        Ok(Scripted::say("Added a comment.")),
    ];
    ask_with_tools(&mut rig, Mode::Edit, turns);
    assert_eq!(
        source(&rig),
        before,
        "the editor is not touched while the assistant works"
    );
    let proposal = rig
        .manager
        .ai
        .proposal
        .clone()
        .expect("a proposal from the draft");
    assert!(proposal.script.contains("// by the assistant"));
    assert_eq!(rig.manager.ai.steps.len(), 2);
    // The trail draws, closed and with one step opened.
    rig.frame();
    rig.manager.cmds.push(Cmd::Ai(PanelAction::Accept));
    rig.frame();
    assert_eq!(source(&rig), proposal.script);
}

#[test]
fn the_trail_draws_while_running_after_it_and_at_a_small_size() {
    let mut rig = Rig::new("ai-agent-draw");
    rig.screen = Some(egui::Rect::from_min_size(
        egui::Pos2::ZERO,
        egui::vec2(700.0, 420.0),
    ));
    open_panel(&mut rig);
    rig.manager.ai.steps = vec![
        ai::Entry::Said("Let me look at the inputs first.".to_owned()),
        ai::Entry::Tool(ai::ToolStep {
            id: "a".to_owned(),
            label: "Reading the script".to_owned(),
            outcome: Some(ai::StepOutcome {
                summary: "Read the script \u{b7} 12 lines".to_owned(),
                ok: true,
                detail: "1: //@version=6\n2: indicator(\"x\")".to_owned(),
            }),
        }),
        ai::Entry::Tool(ai::ToolStep {
            id: "b".to_owned(),
            label: "Editing the script".to_owned(),
            outcome: Some(ai::StepOutcome {
                summary: "`old` matches 2 places (lines 4, 5). Include more of the surrounding lines so it matches once.".to_owned(),
                ok: false,
                detail: String::new(),
            }),
        }),
        ai::Entry::Tool(ai::ToolStep {
            id: "c".to_owned(),
            label: "Compiling the draft".to_owned(),
            outcome: None,
        }),
    ];
    for _ in 0..3 {
        rig.frame();
    }
}

#[test]
fn the_assistant_can_profile_its_draft_on_the_preview_bars_in_ask_mode_too() {
    let mut rig = Rig::new("ai-agent-profile");
    open_panel(&mut rig);
    if rig
        .manager
        .preview
        .debug_feed(model::now_ms(), false)
        .is_none()
    {
        // The preview has not loaded bars in this rig; the tool says so instead (covered in ai_profile).
        return;
    }
    let before = source(&rig);
    let turns = vec![
        Ok(Scripted::call("p1", "profile", serde_json::json!({}))),
        Ok(Scripted::say("Line 5 takes most of the time.")),
    ];
    ask_with_tools(&mut rig, Mode::Ask, turns);
    let ai::Entry::Tool(step) = &rig.manager.ai.steps[0] else {
        panic!("a tool step");
    };
    let outcome = step.outcome.as_ref().expect("it ran");
    assert!(outcome.ok, "{}", outcome.detail);
    assert!(outcome.summary.starts_with("Profiled the draft"));
    assert!(outcome.detail.contains("Hottest lines"));
    assert_eq!(source(&rig), before);
    assert!(rig.manager.ai.proposal.is_none());
    rig.frame();
}

fn plan_turns() -> Vec<Result<Scripted, LlmError>> {
    vec![
        Ok(Scripted::call("r1", "read_script", serde_json::json!({}))),
        Ok(Scripted::call(
            "p1",
            "submit_plan",
            serde_json::json!({
                "summary": "The script has no comment at the top.",
                "steps": [
                    {"title": "Add a comment", "detail": "Say what the script is for.", "lines": "1"},
                    {"title": "Check it compiles", "detail": "Compile the result."},
                ],
                "expected_effect": "No change in behaviour."
            }),
        )),
    ]
}

#[test]
fn a_plan_is_shown_with_its_card_and_changes_nothing() {
    let mut rig = Rig::new("ai-plan-card");
    rig.screen = Some(egui::Rect::from_min_size(
        egui::Pos2::ZERO,
        egui::vec2(700.0, 420.0),
    ));
    open_panel(&mut rig);
    let before = source(&rig);
    ask_with_tools(&mut rig, Mode::Plan, plan_turns());
    let plan = rig.manager.ai.plan.clone().expect("a plan");
    assert_eq!(plan.steps.len(), 2);
    assert_eq!(source(&rig), before);
    assert!(rig.manager.ai.proposal.is_none());
    // The mode cannot be switched under it.
    assert!(!rig.manager.ai.set_mode(Mode::Edit));
    rig.frame();
    rig.frame();
}

#[test]
fn approving_a_plan_switches_to_edit_asks_for_it_and_the_plan_stays_in_the_conversation() {
    let mut rig = Rig::new("ai-plan-approve");
    open_panel(&mut rig);
    ask_with_tools(&mut rig, Mode::Plan, plan_turns());
    rig.manager.cmds.push(Cmd::Ai(PanelAction::ApprovePlan));
    rig.frame();
    // The test key store is empty, so the request itself is refused; what matters is that it was
    // made as an Edit request carrying the implement message, with the plan in the history.
    assert!(rig.manager.ai.plan.is_none());
    assert_eq!(rig.manager.ai.mode, Mode::Edit);
    assert_eq!(rig.manager.ai.input, super::ai_prompt::IMPLEMENT_MESSAGE);
    assert!(rig.manager.ai.notice.as_ref().unwrap().error);
    assert!(matches!(
        rig.manager.ai.turns.last(),
        Some(super::ai_prompt::Turn::Assistant { text, mode: Mode::Plan }) if text.contains("**Add a comment**")
    ));
    // The implementing request then edits the draft as any Edit request does.
    let before = source(&rig);
    let first = before.lines().next().unwrap().to_owned();
    ask_with_tools(
        &mut rig,
        Mode::Edit,
        vec![
            Ok(Scripted::call(
                "e1",
                "edit_script",
                serde_json::json!({"old": first, "new": format!("// A script.\n{first}")}),
            )),
            Ok(Scripted::say("Added the comment.")),
        ],
    );
    assert_eq!(source(&rig), before);
    rig.manager.cmds.push(Cmd::Ai(PanelAction::Accept));
    rig.frame();
    assert!(source(&rig).starts_with("// A script.\n"));
}

#[test]
fn revising_a_plan_puts_the_cursor_in_the_box_and_discarding_drops_it() {
    let mut rig = Rig::new("ai-plan-revise");
    open_panel(&mut rig);
    ask_with_tools(&mut rig, Mode::Plan, plan_turns());
    rig.manager.cmds.push(Cmd::Ai(PanelAction::RevisePlan));
    rig.frame();
    assert!(rig.manager.ai.plan.is_none());
    assert_eq!(
        rig.manager.ai.mode,
        Mode::Plan,
        "revising stays in Plan mode"
    );
    assert!(rig.manager.ai.input.starts_with("Revise the plan: "));
    ask_with_tools(&mut rig, Mode::Plan, plan_turns());
    rig.manager.cmds.push(Cmd::Ai(PanelAction::DiscardPlan));
    rig.frame();
    assert!(rig.manager.ai.plan.is_none());
    assert!(rig.manager.ai.set_mode(Mode::Ask));
}

#[test]
fn approving_with_no_plan_waiting_does_nothing() {
    let mut rig = Rig::new("ai-plan-none");
    open_panel(&mut rig);
    rig.manager.cmds.push(Cmd::Ai(PanelAction::ApprovePlan));
    rig.frame();
    assert_eq!(rig.manager.ai.mode, Mode::Ask);
    assert!(rig.manager.ai.notice.is_none());
}

fn transcript_at_end(rig: &Rig) -> bool {
    rig.ctx
        .data(|data| data.get_temp(egui::Id::new("ai-transcript-at-end")))
        .expect("the transcript was drawn")
}

#[test]
fn a_new_plan_or_proposal_scrolls_into_view_so_its_buttons_can_be_pressed() {
    let mut rig = Rig::new("ai-scroll-end");
    rig.screen = Some(egui::Rect::from_min_size(
        egui::Pos2::ZERO,
        egui::vec2(700.0, 520.0),
    ));
    open_panel(&mut rig);
    ask_with_tools(&mut rig, Mode::Plan, plan_turns());
    for _ in 0..40 {
        rig.frame();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(transcript_at_end(&rig), "the plan card is below the fold");
    rig.manager.ai.clear();
    let before = source(&rig);
    ask(
        &mut rig,
        script_reply(&format!("{before}// by the assistant\n")),
    );
    for _ in 0..40 {
        rig.frame();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        transcript_at_end(&rig),
        "the proposal card is below the fold"
    );
}

// MARK: Auto

fn set_auto(rig: &mut Rig, on: bool) {
    rig.manager.cmds.push(Cmd::Ai(PanelAction::SetAuto(on)));
    rig.frame();
}

/// An edit that adds a comment line at the top: clean, and it compiles.
fn comment_edit(rig: &Rig) -> Scripted {
    let first = source(rig).lines().next().unwrap().to_owned();
    Scripted::call(
        "e1",
        "edit_script",
        serde_json::json!({"old": first, "new": format!("// auto\n{first}")}),
    )
}

#[test]
fn manual_is_the_default_and_keeps_the_review_card() {
    let mut rig = Rig::new("ai-manual-default");
    open_panel(&mut rig);
    assert!(!rig.manager.ai.prefs.auto_apply);
    let before = source(&rig);
    let turns = vec![Ok(comment_edit(&rig)), Ok(Scripted::say("Done."))];
    ask_with_tools(&mut rig, Mode::Edit, turns);
    assert_eq!(source(&rig), before);
    assert!(rig.manager.ai.proposal.is_some());
}

#[test]
fn auto_applies_a_clean_change_and_one_undo_brings_the_script_back() {
    let mut rig = Rig::new("ai-auto-apply");
    open_panel(&mut rig);
    set_auto(&mut rig, true);
    let before = source(&rig);
    let turns = vec![
        Ok(comment_edit(&rig)),
        Ok(Scripted::say("Added a comment.")),
    ];
    ask_with_tools(&mut rig, Mode::Edit, turns);
    rig.frame();
    assert!(source(&rig).starts_with("// auto\n"));
    assert!(rig.manager.ai.proposal.is_none());
    let notice = rig.manager.ai.notice.clone().expect("a notice");
    assert!(!notice.error);
    assert!(
        notice.text.starts_with("Applied automatically"),
        "{}",
        notice.text
    );
    assert!(notice.text.contains("Undo"));
    // It is applied to the editor, not saved.
    assert!(rig.manager.open.as_ref().unwrap().editor.is_dirty());
    rig.frame();
    rig.frame_with(vec![egui::Event::Key {
        key: egui::Key::Z,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::COMMAND,
    }]);
    assert_eq!(
        source(&rig),
        before,
        "one undo brings back the whole change"
    );
}

#[test]
fn auto_holds_a_result_that_does_not_compile() {
    let mut rig = Rig::new("ai-auto-errors");
    open_panel(&mut rig);
    set_auto(&mut rig, true);
    let before = source(&rig);
    let turns = vec![
        Ok(Scripted::call(
            "e1",
            "edit_script",
            serde_json::json!({"old": "plot(value)", "new": "plot(value"}),
        )),
        Ok(Scripted::say("Changed the plot.")),
    ];
    ask_with_tools(&mut rig, Mode::Edit, turns);
    rig.frame();
    assert_eq!(source(&rig), before, "nothing was applied");
    let proposal = rig.manager.ai.proposal.clone().expect("held for review");
    assert!(proposal.errors > 0);
    let note = proposal.note.unwrap();
    assert!(
        note.starts_with("Not applied automatically: it has"),
        "{note}"
    );
}

#[test]
fn auto_holds_a_request_that_did_not_finish() {
    let mut rig = Rig::new("ai-auto-incomplete");
    open_panel(&mut rig);
    set_auto(&mut rig, true);
    let before = source(&rig);
    let turns = vec![
        Ok(comment_edit(&rig)),
        Err(LlmError::rate_limited("slow down")),
    ];
    ask_with_tools(&mut rig, Mode::Edit, turns);
    rig.frame();
    assert_eq!(source(&rig), before);
    let note = rig.manager.ai.proposal.clone().unwrap().note.unwrap();
    assert!(note.starts_with("Incomplete"), "{note}");
    assert!(
        note.ends_with("Not applied automatically: it did not finish."),
        "{note}"
    );
}

#[test]
fn auto_does_not_overwrite_text_typed_while_the_assistant_worked() {
    let mut rig = Rig::new("ai-auto-typed");
    open_panel(&mut rig);
    set_auto(&mut rig, true);
    let turns = vec![Ok(comment_edit(&rig)), Ok(Scripted::say("Done."))];
    begin_with_tools(&mut rig, Mode::Edit, turns);
    let typed = format!("{}// typed meanwhile\n", source(&rig));
    rig.set_source(&typed);
    wait_for_request(&mut rig);
    rig.frame();
    assert_eq!(source(&rig), typed);
    let note = rig.manager.ai.proposal.clone().expect("held").note.unwrap();
    assert!(
        note.contains("the script changed while the assistant worked"),
        "{note}"
    );
}

#[test]
fn auto_leaves_ask_and_plan_alone() {
    let mut rig = Rig::new("ai-auto-ask-plan");
    open_panel(&mut rig);
    set_auto(&mut rig, true);
    let before = source(&rig);
    ask_in(
        &mut rig,
        Mode::Ask,
        script_reply(&format!("{before}// x\n")),
    );
    assert_eq!(source(&rig), before);
    assert!(rig.manager.ai.proposal.is_none());
    ask_with_tools(&mut rig, Mode::Plan, plan_turns());
    assert_eq!(source(&rig), before);
    assert!(rig.manager.ai.plan.is_some());
}

#[test]
fn an_approved_plan_ends_applied_when_auto_is_on() {
    let mut rig = Rig::new("ai-auto-plan");
    open_panel(&mut rig);
    set_auto(&mut rig, true);
    ask_with_tools(&mut rig, Mode::Plan, plan_turns());
    rig.manager.cmds.push(Cmd::Ai(PanelAction::ApprovePlan));
    rig.frame();
    assert_eq!(rig.manager.ai.mode, Mode::Edit);
    // The implementing request (the test key store is empty, so the real one is refused).
    let turns = vec![
        Ok(comment_edit(&rig)),
        Ok(Scripted::say("Added the comment.")),
    ];
    ask_with_tools(&mut rig, Mode::Edit, turns);
    rig.frame();
    assert!(source(&rig).starts_with("// auto\n"));
}

#[test]
fn the_switch_is_saved_on_this_mac_and_survives_a_reload() {
    let mut rig = Rig::new("ai-auto-saved");
    open_panel(&mut rig);
    set_auto(&mut rig, true);
    let saved = AiPrefs::load(&rig.shared.lock().unwrap());
    assert!(saved.auto_apply);
    // Another read of the store (the host does this every second and a half) keeps it.
    rig.manager.ai.prefs = AiPrefs::load(&rig.shared.lock().unwrap());
    assert!(rig.manager.ai.prefs.auto_apply);
    set_auto(&mut rig, false);
    assert!(!AiPrefs::load(&rig.shared.lock().unwrap()).auto_apply);
}

#[test]
fn the_switch_cannot_change_while_a_request_runs() {
    let mut rig = Rig::new("ai-auto-running");
    open_panel(&mut rig);
    let turns = vec![Ok(Scripted::say("thinking"))];
    begin_with_tools(&mut rig, Mode::Edit, turns);
    assert!(rig.manager.ai.is_running());
    let ctx = rig.ctx.clone();
    rig.manager.ai_action(PanelAction::SetAuto(true), &ctx);
    assert!(!rig.manager.ai.prefs.auto_apply);
    wait_for_request(&mut rig);
}

#[test]
fn the_switch_and_its_status_line_draw_in_a_small_panel() {
    let mut rig = Rig::new("ai-auto-draw");
    rig.screen = Some(egui::Rect::from_min_size(
        egui::Pos2::ZERO,
        egui::vec2(700.0, 420.0),
    ));
    open_panel(&mut rig);
    assert!(rig.manager.ai.set_mode(Mode::Edit));
    set_auto(&mut rig, true);
    rig.frame();
    rig.frame();
}

#[test]
fn a_file_edited_outside_makes_a_waiting_proposal_stale_instead_of_being_overwritten() {
    use std::time::{Duration, Instant};
    let mut rig = Rig::new("ai-outside-edit");
    open_panel(&mut rig);
    rig.manager.save_open();
    let before = source(&rig);
    ask(
        &mut rig,
        script_reply(&format!("{before}// by the assistant\n")),
    );
    assert!(rig.manager.ai.proposal.is_some());
    // Another tool rewrites the file, and the app picks it up.
    let id = rig.manager.open.as_ref().unwrap().id.clone();
    let path = rig
        .manager
        .script_store
        .as_mut()
        .unwrap()
        .file_path(&id)
        .unwrap()
        .unwrap();
    let outside = "//@version=6\nindicator(\"Outside\")\nplot(close)\n";
    std::fs::write(path, outside).unwrap();
    let t0 = Instant::now() + Duration::from_secs(10);
    let ctx = rig.ctx.clone();
    for ms in [0, 500, 1000] {
        let now = t0 + Duration::from_millis(ms);
        if rig.manager.external_scan_due(now) {
            rig.manager.refresh();
            rig.manager.reconcile_open(&ctx, now);
        }
    }
    assert_eq!(source(&rig), outside);
    // Applying the proposal now is refused, as it is when the person types: the outside edit stays.
    rig.manager.cmds.push(Cmd::Ai(PanelAction::Accept));
    rig.frame();
    assert_eq!(source(&rig), outside);
    assert!(rig.manager.ai.proposal.is_none());
    assert!(rig.manager.ai.notice.as_ref().unwrap().error);
}
