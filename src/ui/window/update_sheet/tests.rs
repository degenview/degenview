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
    available_with(installable, notes, Coverage::default())
}

fn available_with(installable: bool, notes: ReleaseNotes, coverage: Coverage) -> UpdateView {
    UpdateView {
        phase: Phase::Available {
            version: "0.6.0-rc.33".to_owned(),
            url: "https://example.test/release".to_owned(),
            installable,
            note: (!installable).then(|| {
                "Not published by the release workflow, so it needs a manual download.".to_owned()
            }),
            notes: Arc::new(notes),
            coverage,
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

/// The smallest windows the app opens: a Pine Studio tab window and the Settings window.
const SMALLEST_WINDOWS: [Vec2; 2] = [Vec2::new(620.0, 420.0), Vec2::new(520.0, 480.0)];

/// The sheet's rectangle once it settled in a window of `size`, after `wheeled` frames.
fn settled_rect(size: Vec2, view: &UpdateView) -> egui::Rect {
    let mut screen = Screen::new(size);
    for _ in 0..6 {
        screen.run(vec![], |ui| {
            show(ui.ctx(), view, INSTALLED);
        });
    }
    screen
        .ctx
        .memory(|memory| memory.area_rect(Id::new("update-review")))
        .expect("the sheet is on screen")
}

#[test]
fn the_whole_sheet_fits_the_smallest_windows_whatever_the_list_holds() {
    let manual_note = "Not published by the release workflow, so it needs a manual download, and this build cannot say more than that without wrapping.";
    let mut manual = available(false, flood());
    if let Phase::Available { note, .. } = &mut manual.phase {
        *note = Some(manual_note.to_owned());
    }
    for size in SMALLEST_WINDOWS {
        for (name, view) in [
            ("installable", available(true, flood())),
            ("manual", manual.clone()),
            ("empty", available(true, ReleaseNotes::default())),
        ] {
            let rect = settled_rect(size, &view);
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
            assert!(
                screen.contains_rect(rect),
                "{name} in {size:?}: the sheet {rect:?} leaves the window"
            );
        }
    }
}

#[test]
fn the_list_gets_what_the_window_leaves_and_never_less_than_its_floor() {
    assert_eq!(list_height(2000.0, 200.0), MAX_LIST);
    assert_eq!(list_height(500.0, 230.0), 500.0 - 230.0 - 2.0 * EDGE_MARGIN);
    assert_eq!(list_height(300.0, 280.0), MIN_LIST);
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

/// The text of every label a frame painted.
fn texts(output: &egui::FullOutput) -> Vec<String> {
    output
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
            _ => None,
        })
        .collect()
}

#[test]
fn the_sheet_draws_the_footnotes_the_notes_and_coverage_give() {
    let size = Vec2::new(700.0, 760.0);
    let cut = ReleaseNotes {
        omitted: 3,
        ..notes()
    };
    let coverage = Coverage {
        unsummarized: 2,
        history_incomplete: true,
    };
    let footnotes = cut.footnotes(&coverage);
    assert_eq!(footnotes.len(), 3);
    let view = available_with(true, cut, coverage);
    let drawn = texts(&scrolled(&mut Screen::new(size), &view, &[]));
    for line in &footnotes {
        assert!(drawn.contains(line), "{line} is not drawn in {drawn:?}");
    }

    // Nothing to say about a range that was read in full.
    let plain = available(true, notes());
    let drawn = texts(&scrolled(&mut Screen::new(size), &plain, &[]));
    assert!(
        drawn
            .iter()
            .all(|text| !text.contains("full release notes.") && !text.contains("no summary")),
        "{drawn:?}"
    );

    // With nothing to list the sheet says so and nothing more.
    let empty = available_with(true, ReleaseNotes::default(), coverage);
    let drawn = texts(&scrolled(&mut Screen::new(size), &empty, &[]));
    assert!(
        drawn
            .iter()
            .any(|text| text == "No summary of changes is available for this version.")
    );
    assert!(
        drawn.iter().all(|text| !footnotes.contains(text)),
        "{drawn:?}"
    );
}

/// The sheet in the pseudo-locale, installable and not: its own words went through `tr!` (the
/// notes and the reason a release cannot install are data) and none is cut.
#[test]
fn the_sheet_draws_in_the_pseudo_locale() {
    use crate::i18n::{Locale, with_locale};
    let size = Vec2::new(700.0, 760.0);
    with_locale(Locale::PSEUDO, || {
        for installable in [true, false] {
            let view = available(installable, notes());
            let mut screen = Screen::new(size);
            let mut last = None;
            for _ in 0..4 {
                last = Some(screen.run(vec![], |ui| {
                    show(ui.ctx(), &view, INSTALLED);
                }));
            }
            let frame = last.expect("a frame");
            let words = Screen::words(&frame);
            let notes = notes();
            let data: Vec<&str> = notes
                .new
                .iter()
                .chain(&notes.fixed)
                .map(String::as_str)
                .chain(["Not published by the release workflow"])
                .collect();
            assert_eq!(
                Screen::untranslated(&words, &data),
                Vec::<&str>::new(),
                "installable {installable}: text that did not go through tr!"
            );
            assert_eq!(
                Screen::truncated_text(&frame),
                Vec::<String>::new(),
                "installable {installable}: a label was cut"
            );
        }
    });
}

#[test]
fn updating_from_the_sheet_starts_the_install_and_closes_it() {
    let host = UpdateHost::at_phase(&Phase::Available {
        version: "9.9.9".to_owned(),
        url: "https://example.test/release".to_owned(),
        installable: true,
        note: None,
        notes: Arc::new(notes()),
        coverage: Default::default(),
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
fn a_return_from_before_the_sheet_opened_does_not_confirm_it() {
    let host = UpdateHost::at_phase(&Phase::Available {
        version: "9.9.9".to_owned(),
        url: "https://example.test/release".to_owned(),
        installable: true,
        note: None,
        notes: Arc::new(notes()),
        coverage: Default::default(),
    });
    let mut screen = Screen::new(Vec2::new(700.0, 760.0));
    let root = egui::ViewportId::ROOT;
    host.review(root);
    // The opening frame carries the key that asked for the sheet.
    for events in [vec![key(Key::Enter)], vec![], vec![], vec![]] {
        screen.run(events, |ui| {
            show_review(ui.ctx(), &host, &host.view());
        });
        assert!(
            matches!(host.view().phase, Phase::Available { .. }),
            "nothing installs until a Return on a later frame"
        );
    }
    assert!(host.review_open_in(root), "the sheet is still waiting");
    screen.run(vec![key(Key::Enter)], |ui| {
        show_review(ui.ctx(), &host, &host.view());
    });
    assert!(matches!(host.view().phase, Phase::Installing(_)));
}

#[test]
fn a_return_that_reopens_a_dismissed_sheet_does_not_confirm_it() {
    let host = UpdateHost::at_phase(&Phase::Available {
        version: "9.9.9".to_owned(),
        url: "https://example.test/release".to_owned(),
        installable: true,
        note: None,
        notes: Arc::new(notes()),
        coverage: Default::default(),
    });
    let mut screen = Screen::new(Vec2::new(700.0, 760.0));
    let root = egui::ViewportId::ROOT;
    host.review(root);
    for events in [vec![], vec![], vec![], vec![key(Key::Escape)]] {
        screen.run(events, |ui| {
            show_review(ui.ctx(), &host, &host.view());
        });
    }
    assert!(!host.review_open_in(root), "Escape closed it");
    // A frame without the sheet: it is a new one when it comes back.
    screen.run(vec![], |ui| {
        show_review(ui.ctx(), &host, &host.view());
    });

    // The key press that clicks the tab mark arrives in the frame that draws the sheet again.
    // egui runs no sizing pass for a sheet it has laid out before, so nothing else keeps that
    // press away from the focused button.
    host.review(root);
    for events in [vec![key(Key::Enter)], vec![], vec![], vec![]] {
        screen.run(events, |ui| {
            show_review(ui.ctx(), &host, &host.view());
        });
        assert!(
            matches!(host.view().phase, Phase::Available { .. }),
            "nothing installs until a Return on a later frame"
        );
    }
    assert!(host.review_open_in(root), "the sheet is still waiting");
    screen.run(vec![key(Key::Enter)], |ui| {
        show_review(ui.ctx(), &host, &host.view());
    });
    assert!(matches!(host.view().phase, Phase::Installing(_)));
}

#[test]
fn an_offer_replaced_since_the_sheet_drew_keeps_the_sheet_open_and_installs_nothing() {
    let host = UpdateHost::at_phase(&Phase::Available {
        version: "9.9.9".to_owned(),
        url: "https://example.test/release".to_owned(),
        installable: true,
        note: None,
        notes: Arc::new(notes()),
        coverage: Default::default(),
    });
    // What the sheet drew was an older offer than the host holds now.
    let stale = UpdateView {
        phase: Phase::Available {
            version: "9.9.8".to_owned(),
            url: "https://example.test/old".to_owned(),
            installable: true,
            note: None,
            notes: Arc::new(notes()),
            coverage: Default::default(),
        },
        ..host.view()
    };
    let mut screen = Screen::new(Vec2::new(700.0, 760.0));
    let root = egui::ViewportId::ROOT;
    host.review(root);
    for events in [vec![], vec![], vec![], vec![key(Key::Enter)]] {
        screen.run(events, |ui| {
            show_review(ui.ctx(), &host, &stale);
        });
    }
    assert!(host.review_open_in(root));
    assert!(matches!(host.view().phase, Phase::Available { .. }));
}

fn offer(version: &str) -> Phase {
    Phase::Available {
        version: version.to_owned(),
        url: "https://example.test/release".to_owned(),
        installable: true,
        note: None,
        notes: Arc::new(notes()),
        coverage: Default::default(),
    }
}

#[test]
fn an_offer_that_replaces_the_one_on_screen_is_not_confirmed_in_its_first_frame() {
    let host = UpdateHost::at_phase(&offer("9.9.9"));
    let mut screen = Screen::new(Vec2::new(700.0, 760.0));
    let root = egui::ViewportId::ROOT;
    host.review(root);
    for _ in 0..4 {
        screen.run(vec![], |ui| {
            show_review(ui.ctx(), &host, &host.view());
        });
    }
    // A check puts 9.9.10 on offer; the Return that was meant for 9.9.9 lands in the first frame
    // that shows it.
    host.stand_at(&offer("9.9.10"));
    screen.run(vec![key(Key::Enter)], |ui| {
        show_review(ui.ctx(), &host, &host.view());
    });
    assert!(
        matches!(host.view().phase, Phase::Available { ref version, .. } if version == "9.9.10"),
        "nothing installs"
    );
    assert!(host.review_open_in(root), "the sheet stays open");

    screen.run(vec![key(Key::Enter)], |ui| {
        show_review(ui.ctx(), &host, &host.view());
    });
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
        coverage: Default::default(),
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
/// enough to wrap twice), one that cannot install itself, a release with no notes, and lists that
/// end with how many releases had no summary or that the notes were cut short. Run by hand:
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
    let cut_short = ReleaseNotes {
        truncated: true,
        omitted: 12,
        ..flood()
    };
    let gaps = Coverage {
        unsummarized: 31,
        ..Coverage::default()
    };
    let one_gap = Coverage {
        unsummarized: 1,
        ..Coverage::default()
    };
    let older_unlisted = Coverage {
        history_incomplete: true,
        ..Coverage::default()
    };
    let long_note = "Not published by the release workflow, so it needs a manual download, and this build cannot say more than that without wrapping.";
    let mut manual = available(false, flood());
    if let Phase::Available { note, .. } = &mut manual.phase {
        *note = Some(long_note.to_owned());
    }
    let roomy = Vec2::new(620.0, 700.0);
    // Each picture: its name, what the sheet shows, the window, how far the list was wheeled
    // first, and how many times Tab moved the focus before the picture.
    let cases = [
        ("installable", available(true, notes()), roomy, 0.0, 0),
        ("manual", available(false, notes()), roomy, 0.0, 0),
        (
            "empty",
            available(true, ReleaseNotes::default()),
            roomy,
            0.0,
            0,
        ),
        ("one-list", available(true, only_new), roomy, 0.0, 0),
        (
            "unsummarized",
            available_with(true, notes(), gaps),
            roomy,
            0.0,
            0,
        ),
        (
            "older-unlisted",
            available_with(true, notes(), older_unlisted),
            roomy,
            0.0,
            0,
        ),
        (
            "unsummarized-truncated-end",
            available_with(true, truncated.clone(), one_gap),
            roomy,
            -5000.0,
            0,
        ),
        ("many", available(true, flood()), roomy, 0.0, 0),
        ("many-scrolled", available(true, flood()), roomy, -150.0, 0),
        (
            "many-scrolled-end",
            available(true, flood()),
            roomy,
            -5000.0,
            0,
        ),
        (
            "truncated",
            available(true, truncated.clone()),
            roomy,
            0.0,
            0,
        ),
        (
            "truncated-scrolled-end",
            available(true, truncated),
            roomy,
            -5000.0,
            0,
        ),
        (
            "cut-short-scrolled-end",
            available(true, cut_short),
            roomy,
            -5000.0,
            0,
        ),
        (
            "keyboard-focus-cancel",
            available(true, notes()),
            roomy,
            0.0,
            1,
        ),
        (
            "keyboard-focus-update",
            available(true, notes()),
            roomy,
            0.0,
            4,
        ),
        (
            "small-installable",
            available(true, flood()),
            SMALLEST_WINDOWS[0],
            0.0,
            0,
        ),
        ("small-manual", manual.clone(), SMALLEST_WINDOWS[0], 0.0, 0),
        (
            "small-settings",
            available(true, flood()),
            SMALLEST_WINDOWS[1],
            0.0,
            0,
        ),
        ("small-settings-manual", manual, SMALLEST_WINDOWS[1], 0.0, 0),
    ];
    for light in [false, true] {
        for (name, view, size, wheeled, tabs) in &cases {
            let mut screen = Screen::new(*size);
            screen.ctx.set_theme(if light {
                egui::ThemePreference::Light
            } else {
                egui::ThemePreference::Dark
            });
            let mut last = None;
            let frames = (0..5)
                .map(|_| vec![])
                .chain((0..*tabs).map(|_| vec![key(Key::Tab)]))
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
