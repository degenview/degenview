use super::*;
use crate::model::chart::canvas::BackgroundMode;
use crate::model::chart::kind::ChartType;
use crate::model::chart::template::BUILT_INS;
use crate::store::Store;
use eframe::egui::Rect;
use std::sync::{Arc, Mutex};

fn shared() -> SharedStore {
    Arc::new(Mutex::new(Store::open_in_memory().expect("open")))
}

fn press(key: egui::Key) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

/// Runs the dialogs for a few frames, pressing `key` on the second.
fn run(
    state: &mut TemplateState,
    store: &SharedStore,
    card: &mut ChartCard,
    key: Option<egui::Key>,
) -> Option<TemplateAction> {
    let ctx = egui::Context::default();
    let mut result = None;
    for frame in 0..4 {
        let events = match key {
            Some(key) if frame == 2 => vec![press(key)],
            _ => Vec::new(),
        };
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(1000.0, 800.0))),
            time: Some(f64::from(frame) * 0.1),
            events,
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            if let Some(action) = dialogs(ui.ctx(), state, store, card) {
                result = Some(action);
            }
        });
        output.textures_delta.clear();
    }
    result
}

fn styled_card() -> ChartCard {
    let mut card = ChartCard::default();
    let appearance = card.appearance_mut();
    appearance.bullish = [10, 20, 30];
    appearance.canvas.background.mode = BackgroundMode::Gradient;
    appearance.canvas.margins.right = 33;
    card.set_chart_type(ChartType::Bars);
    card
}

fn open_save(state: &mut TemplateState, name: &str) {
    state.dialog = Some(Dialog::Save {
        name: name.to_owned(),
        description: String::new(),
        focus: false,
        error: None,
    });
}

#[test]
fn saving_a_template_snapshots_the_chart_and_closes_the_dialog() {
    let (store, mut state, mut card) = (shared(), TemplateState::default(), styled_card());
    open_save(&mut state, "My Dark Theme");
    assert!(state.has_dialog());
    run(&mut state, &store, &mut card, Some(egui::Key::Enter));
    assert!(!state.has_dialog(), "saved and closed");
    let saved = ChartTemplateStore::new(store.clone());
    assert_eq!(saved.book().saved().len(), 1);
    assert_eq!(saved.book().saved()[0].name, "My Dark Theme");
    assert_eq!(saved.book().saved()[0].appearance, card.appearance());
    // The chart changing afterwards does not touch what was saved.
    card.appearance_mut().bullish = [1, 1, 1];
    assert_eq!(
        ChartTemplateStore::new(store).book().saved()[0]
            .appearance
            .bullish,
        [10, 20, 30]
    );
    assert!(
        state
            .notice
            .as_ref()
            .is_some_and(|n| !n.error && n.text.contains("Saved"))
    );
}

#[test]
fn a_duplicate_or_empty_name_keeps_the_dialog_open_with_the_reason() {
    let (store, mut state, mut card) = (shared(), TemplateState::default(), styled_card());
    open_save(&mut state, "Same");
    run(&mut state, &store, &mut card, Some(egui::Key::Enter));
    open_save(&mut state, "same");
    run(&mut state, &store, &mut card, Some(egui::Key::Enter));
    assert!(state.has_dialog(), "refused, so the user can fix it");
    let Some(Dialog::Save {
        error: Some(error), ..
    }) = &state.dialog
    else {
        panic!("the reason is shown");
    };
    assert!(error.contains("already exists"), "{error}");
    assert_eq!(
        ChartTemplateStore::new(store.clone()).book().saved().len(),
        1
    );
    // A blank name cannot even be confirmed.
    open_save(&mut state, "   ");
    run(&mut state, &store, &mut card, Some(egui::Key::Enter));
    assert!(state.has_dialog());
    assert_eq!(ChartTemplateStore::new(store).book().saved().len(), 1);
}

#[test]
fn escape_closes_a_dialog_and_changes_nothing() {
    let (store, mut state, mut card) = (shared(), TemplateState::default(), styled_card());
    let before = card.appearance();
    for dialog in [
        Dialog::ResetAppearance,
        Dialog::ApplyToAll,
        Dialog::Manage(Manage::default()),
        Dialog::Save {
            name: "x".into(),
            description: String::new(),
            focus: false,
            error: None,
        },
    ] {
        state.dialog = Some(dialog);
        assert!(run(&mut state, &store, &mut card, Some(egui::Key::Escape)).is_none());
        assert!(!state.has_dialog());
    }
    assert_eq!(card.appearance(), before);
    assert!(ChartTemplateStore::new(store).book().saved().is_empty());
}

#[test]
fn every_dialog_draws() {
    let (store, mut state, mut card) = (shared(), TemplateState::default(), styled_card());
    let id = state
        .templates(&store)
        .save("Kept", "a description", &card.appearance())
        .expect("saved");
    for dialog in [
        Dialog::Save {
            name: String::new(),
            description: String::new(),
            focus: true,
            error: Some("A template with that name already exists.".into()),
        },
        Dialog::Manage(Manage::default()),
        Dialog::Manage(Manage {
            renaming: Some(Renaming {
                id,
                name: "Kept".into(),
                description: String::new(),
                focus: true,
            }),
            error: Some("nope".into()),
        }),
        Dialog::Update(id),
        Dialog::Delete(id),
        Dialog::ApplyToAll,
        Dialog::ResetAppearance,
    ] {
        state.dialog = Some(dialog);
        run(&mut state, &store, &mut card, None);
        assert!(state.has_dialog(), "still waiting for an answer");
    }
}

#[test]
fn resetting_the_appearance_restyles_the_chart_and_keeps_its_scripts() {
    let (store, mut state, mut card) = (shared(), TemplateState::default(), styled_card());
    card.add_pine_instance(None, "RSI", "//@version=6\nindicator(\"x\")\nplot(close)\n");
    card.appearance_mut().axis_decimals = Some(5);
    state.dialog = Some(Dialog::ResetAppearance);
    run(&mut state, &store, &mut card, Some(egui::Key::Enter));
    let after = card.appearance();
    assert_eq!(after.bullish, ChartAppearance::default().bullish);
    assert_eq!(after.canvas, ChartAppearance::default().canvas);
    assert_eq!(after.style.chart_type, ChartType::Candles);
    assert_eq!(after.axis_decimals, Some(5), "the market's precision stays");
    assert_eq!(card.pine_instances().len(), 1);
}

#[test]
fn apply_to_all_asks_the_workspace_with_the_chart_s_look_and_the_chosen_scope() {
    let (store, mut state, mut card) = (shared(), TemplateState::default(), styled_card());
    state.scope = ApplyScope::ColorsAndCanvas;
    state.dialog = Some(Dialog::ApplyToAll);
    let action = run(&mut state, &store, &mut card, Some(egui::Key::Enter));
    assert_eq!(
        action,
        Some(TemplateAction::ApplyToAll(ApplyScope::ColorsAndCanvas))
    );
    assert!(!state.has_dialog());
    // Nothing was applied to this chart itself, and nothing was saved.
    assert_eq!(card.appearance().style.chart_type, ChartType::Bars);
}

#[test]
fn updating_replaces_the_saved_look_only_after_confirming() {
    let (store, mut state, mut card) = (shared(), TemplateState::default(), styled_card());
    let id = state
        .templates(&store)
        .save("Look", "", &card.appearance())
        .expect("saved");
    card.appearance_mut().canvas.margins.top = 40;
    state.dialog = Some(Dialog::Update(id));
    run(&mut state, &store, &mut card, Some(egui::Key::Escape));
    assert_eq!(
        ChartTemplateStore::new(store.clone()).book().saved()[0]
            .appearance
            .canvas
            .margins
            .top,
        10
    );
    state.dialog = Some(Dialog::Update(id));
    run(&mut state, &store, &mut card, Some(egui::Key::Enter));
    let updated = ChartTemplateStore::new(store).book().saved()[0].clone();
    assert_eq!(updated.appearance.canvas.margins.top, 40);
    assert_eq!(updated.name, "Look");
}

#[test]
fn deleting_a_vanished_template_just_closes_the_dialog() {
    let (store, mut state, mut card) = (shared(), TemplateState::default(), styled_card());
    state.dialog = Some(Dialog::Delete(424_242));
    run(&mut state, &store, &mut card, None);
    assert!(!state.has_dialog());
    state.dialog = Some(Dialog::Update(424_242));
    run(&mut state, &store, &mut card, None);
    assert!(!state.has_dialog());
}

#[test]
fn the_save_dialog_suggests_a_free_name() {
    let (store, mut state, mut card) = (shared(), TemplateState::default(), styled_card());
    state
        .templates(&store)
        .save("My Template", "", &card.appearance())
        .expect("saved");
    state.dialog = Some(Dialog::Save {
        name: String::new(),
        description: String::new(),
        focus: true,
        error: None,
    });
    run(&mut state, &store, &mut card, None);
    let Some(Dialog::Save { name, .. }) = &state.dialog else {
        panic!("still open");
    };
    assert_eq!(name, "My Template 2");
}

#[test]
fn the_menu_and_the_notice_draw_with_and_without_templates() {
    let (store, mut state, mut card) = (shared(), TemplateState::default(), styled_card());
    let ctx = egui::Context::default();
    for round in 0..2 {
        if round == 1 {
            state
                .templates(&store)
                .save("Mine", "", &card.appearance())
                .expect("saved");
            state.say("Applied “Mine”.");
        }
        for _ in 0..2 {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                menu(ui, &mut state, &store, &mut card);
                notice(ui, &mut state);
            });
            output.textures_delta.clear();
        }
    }
    assert!(state.notice.is_some());
    // A notice lapses on its own.
    state.notice.as_mut().expect("notice").at =
        Instant::now() - NOTICE_LIFETIME - Duration::from_secs(1);
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| notice(ui, &mut state));
    output.textures_delta.clear();
    assert!(state.notice.is_none());
}

#[test]
fn a_preset_applies_without_any_saved_template() {
    let (store, mut state, mut card) = (shared(), TemplateState::default(), styled_card());
    let entries = state.templates(&store).book().entries();
    assert_eq!(entries.len(), BUILT_INS.len());
    card.apply_template(&entries[0].appearance, state.scope);
    assert_eq!(card.appearance().canvas, entries[0].appearance.canvas);
    assert_eq!(card.appearance().style.chart_type, ChartType::Candles);
}
