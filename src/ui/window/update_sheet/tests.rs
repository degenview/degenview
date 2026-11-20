use super::*;
use crate::ui::render_tests::Screen;
use eframe::egui::{Event, Modifiers, PointerButton, Vec2, pos2};
use std::sync::Arc;
use std::time::Duration;

const INSTALLED: &str = "0.6.0-rc.32";

fn notes() -> ReleaseNotes {
    ReleaseNotes {
        new: vec![
            "The toolbar now ends with the Watchlist button.".to_owned(),
            "Updating shows what is new and fixed before it installs.".to_owned(),
        ],
        fixed: vec![
            "Alpaca stock charts on the free plan no longer hit a data error.".to_owned(),
            long_entry(),
        ],
    }
}

fn long_entry() -> String {
    "Pine scripts no longer misread a loop counter's value from the previous bar when the loop runs a different number of times on each bar, which showed up as indicators that drew a stale level after a symbol switch.".to_owned()
}

fn available(installable: bool, notes: ReleaseNotes) -> UpdateView {
    UpdateView {
        phase: Phase::Available {
            version: "0.6.0-rc.33".to_owned(),
            url: "https://example.test/release".to_owned(),
            installable,
            note: (!installable).then(|| {
                "Not published by the release workflow, so it needs a manual download.".to_owned()
            }),
            notes: Arc::new(notes),
        },
        checking: false,
        checked_ago: Some(Duration::ZERO),
    }
}

fn key(key: egui::Key) -> Event {
    Event::Key {
        key,
        physical_key: Some(key),
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    }
}

fn click(at: egui::Pos2) -> Vec<Event> {
    [true, false]
        .map(|pressed| Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        })
        .into()
}

/// A few frames so the sheet settles, then one with `events`; what that frame answered.
fn answer(view: &UpdateView, events: Vec<Event>) -> Option<Outcome> {
    let mut screen = Screen::new(Vec2::new(700.0, 760.0));
    for _ in 0..3 {
        screen.run(vec![], |ui| {
            show(ui.ctx(), view, INSTALLED);
        });
    }
    let mut outcome = None;
    screen.run(events, |ui| outcome = show(ui.ctx(), view, INSTALLED));
    outcome
}

#[test]
fn nothing_is_drawn_unless_an_update_is_on_offer() {
    let ctx = egui::Context::default();
    for phase in [
        Phase::Unchecked,
        Phase::Checking,
        Phase::UpToDate,
        Phase::CheckFailed("offline".to_owned()),
    ] {
        let view = UpdateView {
            phase,
            checking: false,
            checked_ago: None,
        };
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            assert_eq!(show(ui.ctx(), &view, INSTALLED), None);
        });
        output.textures_delta.clear();
    }
}

#[test]
fn return_updates_and_escape_or_a_click_outside_cancels() {
    let view = available(true, notes());
    assert_eq!(answer(&view, vec![]), None);
    assert_eq!(answer(&view, vec![key(Key::Enter)]), Some(Outcome::Update));
    assert_eq!(answer(&view, vec![key(Key::Escape)]), Some(Outcome::Cancel));
    assert_eq!(
        answer(&view, click(pos2(8.0, 8.0))),
        Some(Outcome::Cancel),
        "the dimmed area around the sheet"
    );
}

#[test]
fn a_release_that_cannot_install_offers_its_page_instead_of_an_update() {
    let view = available(false, notes());
    assert_eq!(
        answer(&view, vec![key(Key::Enter)]),
        Some(Outcome::ViewRelease)
    );
    assert_eq!(answer(&view, vec![key(Key::Escape)]), Some(Outcome::Cancel));
}

#[test]
fn a_sheet_with_a_great_many_changes_stays_within_its_cap() {
    let flood = ReleaseNotes {
        new: (0..60).map(|n| format!("Change number {n}.")).collect(),
        fixed: (0..60).map(|_| long_entry()).collect(),
    };
    let heights = |view: &UpdateView| {
        let mut screen = Screen::new(Vec2::new(700.0, 900.0));
        for _ in 0..4 {
            screen.run(vec![], |ui| {
                show(ui.ctx(), view, INSTALLED);
            });
        }
        screen
            .ctx
            .memory(|memory| memory.area_rect(Id::new("update-review")))
            .expect("the sheet is on screen")
            .height()
    };
    let tall = heights(&available(true, flood));
    let short = heights(&available(true, ReleaseNotes::default()));
    assert!(
        tall > short && tall < short + MAX_LIST + 1.0,
        "{tall} vs {short}"
    );
}

#[test]
fn updating_from_the_sheet_starts_the_install_and_closes_it() {
    let host = UpdateHost::at_phase(&Phase::Available {
        version: "9.9.9".to_owned(),
        url: "https://example.test/release".to_owned(),
        installable: true,
        note: None,
        notes: Arc::new(notes()),
    });
    let mut screen = Screen::new(Vec2::new(700.0, 760.0));
    let root = egui::ViewportId::ROOT;
    host.review(root);
    for events in [vec![], vec![], vec![], vec![key(Key::Enter)]] {
        screen.run(events, |ui| {
            show_review(ui.ctx(), &host, &host.view());
        });
    }
    assert!(!host.review_open_in(root));
    assert!(matches!(host.view().phase, Phase::Installing(_)));
}

#[test]
fn cancelling_closes_the_sheet_and_leaves_the_update_on_offer() {
    let host = UpdateHost::at_phase(&Phase::Available {
        version: "9.9.9".to_owned(),
        url: "https://example.test/release".to_owned(),
        installable: true,
        note: None,
        notes: Arc::new(notes()),
    });
    let mut screen = Screen::new(Vec2::new(700.0, 760.0));
    let root = egui::ViewportId::ROOT;
    host.review(root);
    for events in [vec![], vec![], vec![key(Key::Escape)]] {
        screen.run(events, |ui| {
            show_review(ui.ctx(), &host, &host.view());
        });
    }
    assert!(!host.review_open_in(root));
    assert!(matches!(host.view().phase, Phase::Available { .. }));
}

/// The sheet as pictures, in both themes: an installable update with both lists (one entry long
/// enough to wrap twice), one that cannot install itself, and a release with no notes. Run by hand:
/// `DEGENVIEW_GALLERY_DIR=<dir> cargo test --lib update_sheet_gallery -- --ignored`.
#[test]
#[ignore = "writes pictures; run by hand"]
fn update_sheet_gallery() {
    let _restore = theme::RestoreDefaultTheme;
    let only_new = ReleaseNotes {
        new: vec!["Updating shows what is new and fixed before it installs.".to_owned()],
        fixed: vec![],
    };
    let cases = [
        ("installable", available(true, notes())),
        ("manual", available(false, notes())),
        ("empty", available(true, ReleaseNotes::default())),
        ("one-list", available(true, only_new)),
        (
            "many",
            available(
                true,
                ReleaseNotes {
                    new: (1..=14)
                        .map(|n| format!("A new thing, number {n}."))
                        .collect(),
                    fixed: (1..=8).map(|_| long_entry()).collect(),
                },
            ),
        ),
    ];
    for light in [false, true] {
        for (name, view) in &cases {
            let mut screen = Screen::new(Vec2::new(620.0, 700.0));
            screen.ctx.set_theme(if light {
                egui::ThemePreference::Light
            } else {
                egui::ThemePreference::Dark
            });
            let mut last = None;
            for _ in 0..5 {
                last = Some(screen.run(vec![], |ui| {
                    theme::sync(ui.ctx());
                    ui.painter()
                        .rect_filled(ui.max_rect(), 0.0, theme::background());
                    show(ui.ctx(), view, INSTALLED);
                }));
            }
            let theme = if light { "light" } else { "dark" };
            screen.save(&format!("update-sheet-{name}-{theme}"), last.expect("ran"));
        }
    }
}
