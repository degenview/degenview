use super::*;
use crate::ui::render_tests::{Screen, gallery_dir};
use eframe::egui::{Pos2, vec2};
use std::cell::Cell;

const ORIGIN: Pos2 = pos2(20.0, 20.0);
/// Heading, then the swatches, then the rows: the same arithmetic `show` lays out.
const SWATCH_Y: f32 = 20.0 + 4.0 + 24.0 + SWATCH_ROW / 2.0;
const ROWS_TOP: f32 = 20.0 + 4.0 + 24.0 + SWATCH_ROW;

fn swatch_at(index: usize) -> Pos2 {
    pos2(
        ORIGIN.x + PADDING + SWATCH / 2.0 + index as f32 * PITCH,
        SWATCH_Y,
    )
}

fn row_at(index: usize) -> Pos2 {
    pos2(ORIGIN.x + 120.0, ROWS_TOP + ROW * index as f32 + ROW / 2.0)
}

/// Draws the popover a few frames, clicks `at`, and returns what it said.
fn click(color: Option<LinkColor>, toggles: LinkToggles, at: Pos2) -> Option<TabLinkEdit> {
    let mut screen = Screen::new(vec2(340.0, 320.0));
    let said = Cell::new(None);
    let frame = |screen: &mut Screen, events: Vec<egui::Event>| {
        screen.run(events, |ui| {
            ui.scope_builder(
                egui::UiBuilder::new().max_rect(Rect::from_min_size(ORIGIN, vec2(WIDTH, 300.0))),
                |ui| {
                    if let Some(edit) = show(ui, color, toggles) {
                        said.set(Some(edit));
                    }
                },
            );
        })
    };
    let button = |pressed| egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    for _ in 0..3 {
        frame(&mut screen, Vec::new());
    }
    frame(&mut screen, vec![egui::Event::PointerMoved(at)]);
    frame(&mut screen, vec![button(true)]);
    frame(&mut screen, vec![button(false)]);
    said.get()
}

#[test]
fn there_is_a_swatch_for_every_colour_and_no_two_alike() {
    let mut seen: Vec<Color32> = LinkColor::ALL.into_iter().map(swatch).collect();
    seen.sort_by_key(|color| color.to_array());
    seen.dedup();
    assert_eq!(seen.len(), 9);
}

#[test]
fn the_nine_swatches_fit_the_popover_and_keep_their_hit_targets_apart() {
    let last = swatch_at(8).x + HIT / 2.0;
    assert!(last <= ORIGIN.x + WIDTH - PADDING + 2.0, "{last}");
    const { assert!(PITCH >= HIT, "neighbouring hit targets must not overlap") };
}

#[test]
fn choosing_a_colour_tags_the_tab() {
    let said = click(None, LinkToggles::default(), swatch_at(4));
    assert_eq!(said, Some(TabLinkEdit::Color(Some(LinkColor::Orange))));
    let said = click(Some(LinkColor::Red), LinkToggles::default(), swatch_at(8));
    assert_eq!(said, Some(TabLinkEdit::Color(Some(LinkColor::Violet))));
}

#[test]
fn choosing_the_chosen_colour_unlinks_the_tab() {
    let said = click(
        Some(LinkColor::Orange),
        LinkToggles::default(),
        swatch_at(4),
    );
    assert_eq!(said, Some(TabLinkEdit::Color(None)));
}

#[test]
fn a_row_flips_its_own_switch_and_nothing_else() {
    type Read = fn(LinkToggles) -> bool;
    let rows: [(usize, Read); 4] = [
        (0, |t| t.symbol),
        (1, |t| t.interval),
        (2, |t| t.time),
        (3, |t| t.date_range),
    ];
    for (index, check) in rows {
        let before = LinkToggles::default();
        let said = click(Some(LinkColor::Blue), before, row_at(index));
        let Some(TabLinkEdit::Toggles(after)) = said else {
            panic!("row {index}: {said:?}");
        };
        assert_ne!(check(after), check(before), "row {index} flipped");
        let others = [
            after.symbol != before.symbol,
            after.interval != before.interval,
            after.time != before.time,
            after.date_range != before.date_range,
        ];
        assert_eq!(others.iter().filter(|changed| **changed).count(), 1);
    }
}

#[test]
fn with_no_colour_the_switches_do_nothing() {
    assert_eq!(click(None, LinkToggles::default(), row_at(1)), None);
}

#[test]
fn each_row_explains_itself() {
    let texts: Vec<_> = Row::ALL.into_iter().map(explanation).collect();
    assert!(texts.iter().all(|text| text.ends_with('.')));
    let mut unique = texts.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), 4);
}

#[test]
fn the_group_says_what_it_syncs() {
    assert_eq!(describe(LinkToggles::default()), "symbol");
    assert_eq!(describe(LinkToggles::NONE), "nothing synced");
}

/// The popover as a picture, both themes. Run by hand with `DEGENVIEW_GALLERY_DIR` set.
#[test]
#[ignore = "writes images for a person to look at"]
fn gallery_of_the_tab_linking() {
    if gallery_dir().is_none() {
        return;
    }
    let _restore = theme::RestoreDefaultTheme;
    for (theme_name, theme_choice) in [("dark", egui::Theme::Dark), ("light", egui::Theme::Light)] {
        for (state, color, toggles) in [
            ("linked", Some(LinkColor::Orange), LinkToggles::default()),
            ("unlinked", None, LinkToggles::default()),
            (
                "all-on",
                Some(LinkColor::Yellow),
                LinkToggles {
                    symbol: true,
                    interval: true,
                    time: true,
                    date_range: true,
                },
            ),
        ] {
            let mut screen = Screen::new(vec2(340.0, 280.0));
            screen.ctx.set_theme(theme_choice);
            let mut last = None;
            for _ in 0..6 {
                last = Some(screen.run(Vec::new(), |ui| {
                    theme::sync(ui.ctx());
                    ui.painter()
                        .rect_filled(ui.max_rect(), 0.0, theme::chrome());
                    ui.scope_builder(
                        egui::UiBuilder::new()
                            .max_rect(Rect::from_min_size(ORIGIN, vec2(WIDTH, 250.0))),
                        |ui| {
                            kit::menu_frame().show(ui, |ui| {
                                show(ui, color, toggles);
                            });
                        },
                    );
                }));
            }
            screen.save(
                &format!("tab-link-menu-{state}-{theme_name}"),
                last.expect("a frame"),
            );
        }
    }
}
