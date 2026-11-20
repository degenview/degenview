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
        ..ReleaseNotes::default()
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
        ..ReleaseNotes::default()
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

fn flood() -> ReleaseNotes {
    ReleaseNotes {
        new: (1..=14)
            .map(|n| format!("A new thing, number {n}."))
            .collect(),
        fixed: (1..=8).map(|_| long_entry()).collect(),
        ..ReleaseNotes::default()
    }
}

/// A turn of the mouse wheel over the list, `delta` points (negative scrolls down).
fn wheel(delta: f32) -> Vec<Event> {
    vec![
        Event::PointerMoved(pos2(310.0, 400.0)),
        Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: Vec2::new(0.0, delta),
            phase: egui::TouchPhase::Move,
            modifiers: Modifiers::NONE,
        },
    ]
}

/// Frames of the sheet over `view`, the last of them after the notes were wheeled by each of
/// `wheels` and the scrolling settled.
fn scrolled(screen: &mut Screen, view: &UpdateView, wheels: &[f32]) -> egui::FullOutput {
    let mut frames = vec![vec![]; 3];
    frames.extend(wheels.iter().map(|&delta| wheel(delta)));
    frames.extend(vec![vec![]; 20]);
    let mut last = None;
    for events in frames {
        last = Some(screen.run(events, |ui| {
            show(ui.ctx(), view, INSTALLED);
        }));
    }
    last.expect("ran")
}

/// How many edge fades a frame painted over the list.
fn fades(output: &egui::FullOutput) -> usize {
    output
        .shapes
        .iter()
        .filter(|clipped| match &clipped.shape {
            egui::Shape::Mesh(mesh) => mesh.vertices.len() == kit::scroll::FADE_VERTICES,
            _ => false,
        })
        .count()
}

#[test]
fn the_list_fades_where_more_lies_beyond_the_visible_part() {
    let size = Vec2::new(700.0, 760.0);
    let view = available(true, flood());
    let top = scrolled(&mut Screen::new(size), &view, &[]);
    assert_eq!(fades(&top), 1, "only the bottom while at the top");
    let middle = scrolled(&mut Screen::new(size), &view, &[-150.0]);
    assert_eq!(fades(&middle), 2, "both ends part-way down");
    let end = scrolled(&mut Screen::new(size), &view, &[-5000.0]);
    assert_eq!(fades(&end), 1, "only the top once the end is reached");

    let short = available(true, notes());
    assert_eq!(fades(&scrolled(&mut Screen::new(size), &short, &[])), 0);
    let none = available(true, ReleaseNotes::default());
    assert_eq!(fades(&scrolled(&mut Screen::new(size), &none, &[])), 0);
}

#[test]
fn the_line_that_ends_a_cut_list_counts_what_was_left_out() {
    assert_eq!(
        omitted_line(1),
        "And 1 more change in the full release notes."
    );
    assert_eq!(
        omitted_line(42),
        "And 42 more changes in the full release notes."
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
        ..ReleaseNotes::default()
    };
    let truncated = ReleaseNotes {
        omitted: 49,
        ..flood()
    };
    // Each picture: its name, what the sheet shows, and how far the list was wheeled first.
    let cases = [
        ("installable", available(true, notes()), 0.0),
        ("manual", available(false, notes()), 0.0),
        ("empty", available(true, ReleaseNotes::default()), 0.0),
        ("one-list", available(true, only_new), 0.0),
        ("many", available(true, flood()), 0.0),
        ("many-scrolled", available(true, flood()), -150.0),
        ("many-scrolled-end", available(true, flood()), -5000.0),
        ("truncated", available(true, truncated.clone()), 0.0),
        (
            "truncated-scrolled-end",
            available(true, truncated),
            -5000.0,
        ),
    ];
    for light in [false, true] {
        for (name, view, wheeled) in &cases {
            let mut screen = Screen::new(Vec2::new(620.0, 700.0));
            screen.ctx.set_theme(if light {
                egui::ThemePreference::Light
            } else {
                egui::ThemePreference::Dark
            });
            let mut last = None;
            let frames = (0..5)
                .map(|_| vec![])
                .chain([wheel(*wheeled)].into_iter().filter(|_| *wheeled != 0.0))
                .chain((0..20).map(|_| vec![]));
            for events in frames {
                last = Some(screen.run(events, |ui| {
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
