use super::*;
use crate::ui::render_tests::{Screen, gallery_dir};
use eframe::egui::{Pos2, Vec2, vec2};
use std::cell::Cell;

const ORIGIN: Pos2 = pos2(20.0, 20.0);
const SCREEN: Vec2 = vec2(380.0, 460.0);

/// Draws the section a few frames, then clicks the centre of the widget `target` and returns what
/// the section said (and whether it asked to close the menu it was in).
fn click(color: Option<LinkColor>, toggles: LinkToggles, target: Id) -> Option<TabLinkEdit> {
    let mut screen = Screen::new(SCREEN);
    let said = Cell::new(None);
    let frame = |screen: &mut Screen, events: Vec<egui::Event>| {
        screen.run(events, |ui| {
            ui.scope_builder(
                egui::UiBuilder::new().max_rect(Rect::from_min_size(ORIGIN, vec2(WIDTH, 420.0))),
                |ui| {
                    if let Some(edit) = show(ui, color, toggles) {
                        said.set(Some(edit));
                    }
                },
            );
        })
    };
    for _ in 0..3 {
        frame(&mut screen, Vec::new());
    }
    let at = screen
        .ctx
        .read_response(target)
        .expect("the widget is drawn")
        .rect
        .center();
    let button = |pressed| egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    frame(&mut screen, vec![egui::Event::PointerMoved(at)]);
    frame(&mut screen, vec![button(true)]);
    frame(&mut screen, vec![button(false)]);
    said.get()
}

fn swatch_id(color: LinkColor) -> Id {
    Id::new(("tab-link-swatch", color.key()))
}

fn option_id(row: Row) -> Id {
    Id::new(("tab-link-option", row.title()))
}

#[test]
fn there_is_a_swatch_for_every_colour_and_no_two_alike() {
    let mut seen: Vec<Color32> = LinkColor::ALL.into_iter().map(swatch).collect();
    seen.sort_by_key(|color| color.to_array());
    seen.dedup();
    assert_eq!(seen.len(), 9);
}

#[test]
fn choosing_a_colour_tags_the_tab() {
    let said = click(None, LinkToggles::default(), swatch_id(LinkColor::Orange));
    assert_eq!(said, Some(TabLinkEdit::Color(Some(LinkColor::Orange))));
    let said = click(
        Some(LinkColor::Red),
        LinkToggles::default(),
        swatch_id(LinkColor::Violet),
    );
    assert_eq!(said, Some(TabLinkEdit::Color(Some(LinkColor::Violet))));
}

#[test]
fn choosing_the_chosen_colour_unlinks_the_tab() {
    let said = click(
        Some(LinkColor::Orange),
        LinkToggles::default(),
        swatch_id(LinkColor::Orange),
    );
    assert_eq!(said, Some(TabLinkEdit::Color(None)));
}

#[test]
fn a_card_flips_its_own_switch_and_nothing_else() {
    type Read = fn(LinkToggles) -> bool;
    let rows: [(Row, Read); 4] = [
        (Row::Symbol, |t| t.symbol),
        (Row::Interval, |t| t.interval),
        (Row::Time, |t| t.time),
        (Row::DateRange, |t| t.date_range),
    ];
    for (row, check) in rows {
        let before = LinkToggles::default();
        let said = click(Some(LinkColor::Blue), before, option_id(row));
        let Some(TabLinkEdit::Toggles(after)) = said else {
            panic!("{row:?}: {said:?}");
        };
        assert_ne!(check(after), check(before), "{row:?} flipped");
        let changed = [
            after.symbol != before.symbol,
            after.interval != before.interval,
            after.time != before.time,
            after.date_range != before.date_range,
        ];
        assert_eq!(changed.iter().filter(|changed| **changed).count(), 1);
    }
}

#[test]
fn unlink_tab_unlinks_and_does_nothing_when_there_is_nothing_to_unlink() {
    let button = Id::new("tab-link-unlink");
    let said = click(Some(LinkColor::Pink), LinkToggles::default(), button);
    assert_eq!(said, Some(TabLinkEdit::Color(None)));
    assert_eq!(click(None, LinkToggles::default(), button), None);
}

#[test]
fn with_no_colour_the_switches_do_nothing() {
    assert_eq!(
        click(None, LinkToggles::default(), option_id(Row::Interval)),
        None
    );
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
            let mut screen = Screen::new(vec2(380.0, 470.0));
            screen.ctx.set_theme(theme_choice);
            let mut last = None;
            for _ in 0..6 {
                last = Some(screen.run(Vec::new(), |ui| {
                    theme::sync(ui.ctx());
                    ui.painter()
                        .rect_filled(ui.max_rect(), 0.0, theme::chrome());
                    ui.scope_builder(
                        egui::UiBuilder::new()
                            .max_rect(Rect::from_min_size(ORIGIN, vec2(WIDTH, 430.0))),
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
