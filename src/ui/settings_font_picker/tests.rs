use super::*;

fn fonts() -> Vec<String> {
    ["Fira Code", "JetBrains Mono", "Menlo", "SF Mono"]
        .map(String::from)
        .to_vec()
}

#[test]
fn the_default_comes_first_until_something_is_typed() {
    let installed = fonts();
    let all = choices("", &installed, None, true);
    assert_eq!(all[0], Choice::Default);
    assert_eq!(all.len(), 5);
    let typed = choices("mono", &installed, None, true);
    assert_eq!(
        typed,
        [Choice::Family("JetBrains Mono"), Choice::Family("SF Mono")]
    );
    assert_eq!(
        choices("", &installed, None, false)[0],
        Choice::Family("Fira Code")
    );
}

#[test]
fn search_ignores_case_and_surrounding_space() {
    let installed = fonts();
    assert_eq!(
        choices("  MENLO ", &installed, None, true),
        [Choice::Family("Menlo")]
    );
    assert!(choices("zzz", &installed, None, true).is_empty());
}

#[test]
fn a_chosen_font_that_is_not_installed_stays_in_the_list() {
    let installed = fonts();
    let all = choices("", &installed, Some("Gone Mono"), true);
    assert_eq!(all.last(), Some(&Choice::Missing("Gone Mono")));
    // An installed one is not listed twice, whatever its case.
    let all = choices("", &installed, Some("menlo"), true);
    assert!(
        !all.iter()
            .any(|choice| matches!(choice, Choice::Missing(_)))
    );
    // And it follows the search like any other.
    assert!(
        choices("fira", &installed, Some("Gone Mono"), true)
            .iter()
            .all(|choice| !matches!(choice, Choice::Missing(_)))
    );
    assert!(
        choices("gone", &installed, Some("Gone Mono"), true)
            .iter()
            .any(|choice| matches!(choice, Choice::Missing(_)))
    );
}

#[test]
fn the_highlight_stops_at_the_ends() {
    assert_eq!(step(0, -1, 5), 0);
    assert_eq!(step(0, 1, 5), 1);
    assert_eq!(step(4, 1, 5), 4);
    assert_eq!(step(3, 1, 0), 0);
}

#[test]
fn the_picker_draws_and_opens_without_trouble() {
    let ctx = egui::Context::default();
    crate::ui::theme::install_offscreen(&ctx);
    let mut state = FontPickerState::default();
    let installed = fonts();
    for frame in 0..4 {
        let mut output = ctx.run_ui(
            egui::RawInput {
                time: Some(f64::from(frame) * 0.1),
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::pos2(0.0, 0.0),
                    vec2(500.0, 400.0),
                )),
                ..Default::default()
            },
            |ui| {
                let _ = show(
                    ui,
                    &mut state,
                    "test",
                    Some("Menlo"),
                    &FontStatus::Using("Menlo".into()),
                    &installed,
                    true,
                    true,
                );
            },
        );
        output.textures_delta.clear();
    }
}
