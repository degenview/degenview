use super::geometry::slot_at;
use super::*;
use crate::host::update::UpdateView;
use crate::model::tab::title::{Detail, TabChart, TabTitleStyle};
use crate::service::watchlist_quotes::WatchlistQuoteBook;
use crate::ui::icons::Icon;
use crate::ui::theme;
use eframe::egui::{self, Color32, Id, Pos2, Rect, pos2, vec2};

#[test]
fn tabs_share_the_room_between_the_limits() {
    assert_eq!(tab_width(1, 2000.0), MAX_TAB_WIDTH);
    assert_eq!(tab_width(40, 800.0), MIN_TAB_WIDTH);
    let width = tab_width(4, 800.0);
    assert!(width > MIN_TAB_WIDTH && width < MAX_TAB_WIDTH);
    // Four tabs and the + fit the strip.
    let used = tab_left(4, width) + NEW_BUTTON_WIDTH;
    assert!(used <= 800.0 + 0.01, "{used}");
}

/// A right-click on a chart tab opens the menu with the Tab Linking section at the bottom
/// (after a separator), and its swatches and switches say what changed for that tab.
#[test]
fn the_tab_menu_ends_with_the_tab_linking_section() {
    use crate::model::tab::link::{LinkColor, LinkToggles};
    let ctx = egui::Context::default();
    let quotes = WatchlistQuoteBook::default();
    let tabs = [
        TabView {
            id: 7,
            title: "BTC".to_owned(),
            icon: Icon::Plus,
            chart: None,
            timeframe: "1D",
            link: Some(LinkColor::Orange),
            linkable: true,
            link_toggles: LinkToggles::default(),
        },
        TabView {
            id: 8,
            title: "Portfolio".to_owned(),
            icon: Icon::Plus,
            chart: None,
            timeframe: "",
            link: None,
            linkable: false,
            link_toggles: LinkToggles::default(),
        },
    ];
    let mut time = 0.0;
    let mut events = Vec::new();
    let mut frame = |ctx: &egui::Context, input: Vec<egui::Event>| {
        time += 0.05;
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 700.0))),
            time: Some(time),
            events: input,
            ..Default::default()
        };
        let mut output = ctx.run_ui(raw, |ui| {
            let shown = TabBar {
                window: 1,
                tabs: &tabs,
                active: 7,
                drag: DragView::default(),
                other_windows: &[],
                quotes: &quotes,
                style: TabTitleStyle::default(),
                update: None,
            }
            .show(ui);
            events.extend(shown.events);
        });
        output.textures_delta.clear();
    };
    let button =
        |at: Pos2, button: egui::PointerButton, pressed: bool| egui::Event::PointerButton {
            pos: at,
            button,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
    for _ in 0..3 {
        frame(&ctx, Vec::new());
    }
    let on_tab = pos2(tab_left(0, tab_width(2, 900.0)) + 120.0, 14.0);
    frame(&ctx, vec![egui::Event::PointerMoved(on_tab)]);
    frame(
        &ctx,
        vec![button(on_tab, egui::PointerButton::Secondary, true)],
    );
    frame(
        &ctx,
        vec![button(on_tab, egui::PointerButton::Secondary, false)],
    );
    for _ in 0..4 {
        frame(&ctx, Vec::new());
    }

    let swatch = |color: LinkColor| {
        ctx.read_response(Id::new(("tab-link-swatch", color.key())))
            .map(|response| response.rect)
    };
    let blue = swatch(LinkColor::Blue).expect("the section is in the menu");
    let move_row = ctx.read_response(Id::new(("tab-link-option", "Symbol")));
    assert!(move_row.is_some(), "the switches are there too");
    // It is the last thing in the menu, under the tab rows.
    assert!(blue.top() > on_tab.y + 150.0, "{blue:?}");

    let at = blue.center();
    frame(&ctx, vec![egui::Event::PointerMoved(at)]);
    frame(&ctx, vec![button(at, egui::PointerButton::Primary, true)]);
    frame(&ctx, vec![button(at, egui::PointerButton::Primary, false)]);
    frame(&ctx, Vec::new());
    assert!(
        events.contains(&TabBarEvent::SetLink {
            tab: 7,
            color: Some(LinkColor::Blue)
        }),
        "{events:?}"
    );
}

/// The tab menu as a picture, both themes. Run by hand with `DEGENVIEW_GALLERY_DIR` set.
#[test]
#[ignore = "writes images for a person to look at"]
fn gallery_of_the_tab_menu() {
    use crate::model::tab::link::{LinkColor, LinkToggles};
    use crate::ui::render_tests::{Screen, gallery_dir};
    if gallery_dir().is_none() {
        return;
    }
    let _restore = theme::RestoreDefaultTheme;
    let quotes = WatchlistQuoteBook::default();
    for (name, choice) in [("dark", egui::Theme::Dark), ("light", egui::Theme::Light)] {
        for (state, link) in [("linked", Some(LinkColor::Orange)), ("unlinked", None)] {
            let tabs = [
                TabView {
                    id: 7,
                    title: "BTCUSDT".to_owned(),
                    icon: Icon::Plus,
                    chart: Some(TabChart {
                        source: crate::model::DataSource::Binance,
                        symbol: "BTCUSDT".to_owned(),
                        title: "BTC/USDT".to_owned(),
                        layout: None,
                        custom_title: None,
                    }),
                    timeframe: "1D",
                    link,
                    linkable: true,
                    link_toggles: LinkToggles::default(),
                },
                TabView {
                    id: 8,
                    title: "Other".to_owned(),
                    icon: Icon::Plus,
                    chart: None,
                    timeframe: "1D",
                    link: Some(LinkColor::Blue),
                    linkable: true,
                    link_toggles: LinkToggles::default(),
                },
            ];
            let mut screen = Screen::new(vec2(520.0, 760.0));
            screen.ctx.set_theme(choice);
            let on_tab = pos2(tab_left(0, tab_width(2, 520.0)) + 100.0, 14.0);
            let button = |pressed| egui::Event::PointerButton {
                pos: on_tab,
                button: egui::PointerButton::Secondary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            let run = |screen: &mut Screen, events: Vec<egui::Event>| {
                screen.run(events, |ui| {
                    theme::sync(ui.ctx());
                    TabBar {
                        window: 1,
                        tabs: &tabs,
                        active: 7,
                        drag: DragView::default(),
                        other_windows: &[],
                        quotes: &quotes,
                        style: TabTitleStyle::default(),
                        update: None,
                    }
                    .show(ui);
                })
            };
            for _ in 0..3 {
                run(&mut screen, Vec::new());
            }
            run(&mut screen, vec![egui::Event::PointerMoved(on_tab)]);
            run(&mut screen, vec![button(true)]);
            run(&mut screen, vec![button(false)]);
            let mut last = None;
            for _ in 0..5 {
                last = Some(run(&mut screen, Vec::new()));
            }
            screen.save(&format!("tab-menu-{state}-{name}"), last.expect("a frame"));
        }
    }
}

/// A tab that cannot be linked (the portfolio, Pine Studio) has no such section.
#[test]
fn a_tab_that_cannot_be_linked_has_no_section() {
    use crate::model::tab::link::LinkColor;
    let ctx = egui::Context::default();
    let quotes = WatchlistQuoteBook::default();
    let tabs = [TabView {
        id: 8,
        title: "Portfolio".to_owned(),
        icon: Icon::Plus,
        chart: None,
        timeframe: "",
        link: None,
        linkable: false,
        link_toggles: Default::default(),
    }];
    let mut time = 0.0;
    let mut frame = |input: Vec<egui::Event>| {
        time += 0.05;
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 700.0))),
            time: Some(time),
            events: input,
            ..Default::default()
        };
        let mut output = ctx.run_ui(raw, |ui| {
            TabBar {
                window: 1,
                tabs: &tabs,
                active: 8,
                drag: DragView::default(),
                other_windows: &[],
                quotes: &quotes,
                style: TabTitleStyle::default(),
                update: None,
            }
            .show(ui);
        });
        output.textures_delta.clear();
    };
    let on_tab = pos2(tab_left(0, tab_width(1, 900.0)) + 120.0, 14.0);
    let button = |pressed| egui::Event::PointerButton {
        pos: on_tab,
        button: egui::PointerButton::Secondary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    for _ in 0..3 {
        frame(Vec::new());
    }
    frame(vec![egui::Event::PointerMoved(on_tab)]);
    frame(vec![button(true)]);
    frame(vec![button(false)]);
    for _ in 0..4 {
        frame(Vec::new());
    }
    assert!(
        ctx.read_response(Id::new(("tab-link-swatch", LinkColor::Blue.key())))
            .is_none()
    );
}

/// The strip the tabs get for a window, with or without an update mark.
fn strip_with(update: Option<&UpdateView>) -> Rect {
    let ctx = egui::Context::default();
    let mut strip = Rect::NOTHING;
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        let tabs = [TabView {
            id: 1,
            title: "BTC".to_owned(),
            icon: Icon::Plus,
            chart: None,
            timeframe: "1D",
            link: None,
            linkable: false,
            link_toggles: Default::default(),
        }];
        let quotes = WatchlistQuoteBook::default();
        strip = TabBar {
            window: 1,
            tabs: &tabs,
            active: 1,
            drag: DragView::default(),
            other_windows: &[],
            quotes: &quotes,
            style: TabTitleStyle::default(),
            update,
        }
        .show(ui)
        .strip;
    });
    output.textures_delta.clear();
    strip
}

/// One frame of a strip `width` wide holding `count` tabs (every other one a chart tab whose
/// logo needs no network), with the tab at `active` showing. Returns the shapes painted.
fn paint_strip(
    count: usize,
    active: usize,
    width: f32,
    style: TabTitleStyle,
) -> Vec<egui::epaint::ClippedShape> {
    paint_strip_linked(count, active, width, style, None)
}

/// [`paint_strip`] with the tab at `linked` tagged Orange.
fn paint_strip_linked(
    count: usize,
    active: usize,
    width: f32,
    style: TabTitleStyle,
    linked: Option<usize>,
) -> Vec<egui::epaint::ClippedShape> {
    let ctx = egui::Context::default();
    let tabs: Vec<TabView> = (0..count)
        .map(|index| TabView {
            id: index as u64 + 1,
            title: format!("Tab {index}"),
            icon: Icon::Plus,
            chart: (index % 2 == 0).then(|| TabChart {
                source: crate::model::DataSource::CoinMarketCap,
                symbol: "A-VERY-LONG-SYMBOL-NAME/USDT".to_owned(),
                title: "A-VERY-LONG-SYMBOL-NAME/USDT".to_owned(),
                layout: Some("A layout with a rather long name".to_owned()),
                custom_title: None,
            }),
            timeframe: "1D",
            link: (linked == Some(index)).then_some(crate::model::tab::link::LinkColor::Orange),
            linkable: false,
            link_toggles: Default::default(),
        })
        .collect();
    let quotes = WatchlistQuoteBook::default();
    let input = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(width, HEIGHT))),
        ..Default::default()
    };
    let mut output = ctx.run_ui(input, |ui| {
        TabBar {
            window: 1,
            tabs: &tabs,
            active: active as u64 + 1,
            drag: DragView::default(),
            other_windows: &[],
            quotes: &quotes,
            style,
            update: None,
        }
        .show(ui);
    });
    output.textures_delta.clear();
    output.shapes
}

/// Rects filled with `color`: the lines under tabs.
fn lines_of(shapes: &[egui::epaint::ClippedShape], color: Color32) -> usize {
    shapes
        .iter()
        .filter(|clipped| matches!(&clipped.shape, egui::Shape::Rect(rect) if rect.fill == color))
        .count()
}

#[test]
fn a_tab_has_no_underline_unless_it_is_linked_and_then_it_is_its_colour() {
    let orange =
        crate::ui::window::tab_link_menu::swatch(crate::model::tab::link::LinkColor::Orange);
    for style in TabTitleStyle::ALL {
        for width in [1400.0, 700.0, 300.0] {
            let plain = paint_strip(4, 1, width, style);
            assert_eq!(lines_of(&plain, theme::SYSTEM_ACCENT), 0, "no blue line");
            assert_eq!(
                lines_of(&plain, orange),
                0,
                "unlinked: none, showing or not"
            );
            // Linked: its colour, whether it is the showing tab or not.
            for active in [1, 2] {
                let linked = paint_strip_linked(4, active, width, style, Some(2));
                assert_eq!(lines_of(&linked, orange), 1, "{style:?} at {width}");
                assert_eq!(lines_of(&linked, theme::SYSTEM_ACCENT), 0);
            }
        }
    }
}

fn tab_separators(shapes: &[egui::epaint::ClippedShape]) -> usize {
    shapes
        .iter()
        .filter(|clipped| match &clipped.shape {
            egui::Shape::LineSegment { points, stroke } => {
                stroke.color == theme::separator()
                    && points[0].x == points[1].x
                    && (points[1].y - points[0].y).abs() == SEPARATOR_HEIGHT
            }
            _ => false,
        })
        .count()
}

#[test]
fn a_hairline_divides_two_tabs_only_when_neither_is_showing() {
    // Five tabs: four gaps. The showing tab takes the gaps on both its sides.
    assert_eq!(
        tab_separators(&paint_strip(5, 0, 1400.0, TabTitleStyle::Full)),
        3
    );
    assert_eq!(
        tab_separators(&paint_strip(5, 2, 1400.0, TabTitleStyle::Full)),
        2
    );
    assert_eq!(
        tab_separators(&paint_strip(5, 4, 1400.0, TabTitleStyle::Full)),
        3
    );
    // A lone tab, or one next to nothing else, has none; two tabs with one showing too.
    assert_eq!(
        tab_separators(&paint_strip(1, 0, 1400.0, TabTitleStyle::Full)),
        0
    );
    assert_eq!(
        tab_separators(&paint_strip(2, 0, 1400.0, TabTitleStyle::Full)),
        0
    );
    assert_eq!(
        tab_separators(&paint_strip(3, 1, 1400.0, TabTitleStyle::Full)),
        0
    );
}

#[test]
fn crowded_and_long_titles_paint_at_every_width() {
    // Nothing panics from a roomy strip down to one that holds only a sliver, with a
    // symbol and a layout name far longer than any tab.
    for count in [1, 2, 12, 60] {
        for width in [2000.0, 900.0, 400.0, 120.0] {
            let shapes = paint_strip(count, count - 1, width, TabTitleStyle::Full);
            assert!(!shapes.is_empty());
        }
    }
}

#[test]
fn tabs_squeeze_to_their_logo_before_they_run_out_of_strip() {
    // Eleven tabs used to overflow a 1000-wide strip at the old 96-wide minimum.
    let width = tab_width(11, 1000.0);
    assert!((MIN_TAB_WIDTH..96.0).contains(&width), "{width}");
    assert_eq!(Detail::for_width(width), Detail::Logo);
    // The logo and the close button fit side by side at the narrowest.
    assert!(
        close_rect(Rect::from_min_size(
            pos2(0.0, 0.0),
            vec2(MIN_TAB_WIDTH, 36.0)
        ))
        .left()
            >= 9.0 + LOGO_SIZE
    );
}

#[test]
fn tabs_scroll_only_when_they_cannot_all_be_shown() {
    assert_eq!(max_scroll(0, 800.0), 0.0);
    assert_eq!(max_scroll(3, 800.0), 0.0);
    // Twenty at the narrowest do not fit 800 wide.
    let max = max_scroll(20, 800.0);
    assert!(max > 0.0);
    // Scrolled to the end, the last tab and the `+` end at the strip's edge.
    let width = tab_width(20, 800.0);
    assert_eq!(width, MIN_TAB_WIDTH);
    let end = tab_left(20, width) - max + NEW_BUTTON_WIDTH;
    assert!((end - 800.0).abs() < 0.01, "{end}");
}

#[test]
fn the_active_tab_is_scrolled_into_view_and_a_visible_one_is_left_alone() {
    let (count, strip) = (20, 800.0);
    let max = max_scroll(count, strip);
    let width = tab_width(count, strip);
    let room = strip - NEW_BUTTON_WIDTH;
    // The last tab, from the start.
    let to_end = reveal(count - 1, count, strip, 0.0);
    assert_eq!(to_end, max);
    // The first, from the end.
    assert_eq!(reveal(0, count, strip, max), 0.0);
    // One already whole in view does not move the strip.
    assert_eq!(reveal(2, count, strip, 10.0), 10.0);
    // Whichever tab is revealed ends up inside the visible part.
    for index in 0..count {
        for from in [0.0, max / 2.0, max] {
            let scroll = reveal(index, count, strip, from);
            let left = tab_left(index, width) - scroll;
            assert!(
                left >= -0.01 && left + width <= room + 0.01,
                "{index} from {from}"
            );
        }
    }
}

#[test]
fn a_scrolled_strip_still_paints_and_reports_where_it_is() {
    let ctx = egui::Context::default();
    let tabs: Vec<TabView> = (0..30)
        .map(|index| TabView {
            id: index as u64 + 1,
            title: format!("Tab {index}"),
            icon: Icon::Plus,
            chart: None,
            timeframe: "1D",
            link: None,
            linkable: false,
            link_toggles: Default::default(),
        })
        .collect();
    let quotes = WatchlistQuoteBook::default();
    let mut scrolls = Vec::new();
    for _ in 0..3 {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(700.0, HEIGHT))),
                ..Default::default()
            },
            |ui| {
                let output = TabBar {
                    window: 1,
                    tabs: &tabs,
                    // The last tab is the active one, so the strip scrolls to it.
                    active: 30,
                    drag: DragView::default(),
                    other_windows: &[],
                    quotes: &quotes,
                    style: TabTitleStyle::default(),
                    update: None,
                }
                .show(ui);
                scrolls.push(output.scroll);
            },
        );
        output.textures_delta.clear();
    }
    assert!(scrolls[0] <= *scrolls.last().unwrap());
    assert!(*scrolls.last().unwrap() > 0.0, "{scrolls:?}");
    assert!(*scrolls.last().unwrap() <= max_scroll(30, 700.0));
}

#[test]
fn the_strip_gives_way_to_an_update_mark_only_while_there_is_one() {
    use crate::host::update::Phase;
    let view = |phase| UpdateView {
        phase,
        checking: false,
        checked_ago: None,
    };
    let none = strip_with(None);
    let quiet = strip_with(Some(&view(Phase::UpToDate)));
    let available = strip_with(Some(&view(Phase::Available {
        version: "9.9.9".to_owned(),
        url: String::new(),
        installable: true,
        note: None,
    })));
    assert_eq!(quiet, none);
    assert!(available.right() < none.right() - 40.0, "{available:?}");
    assert_eq!(available.left(), none.left());
}

#[test]
fn slot_at_clamps_to_the_strip() {
    let width = 100.0;
    assert_eq!(slot_at(-50.0, 3, width), 0);
    assert_eq!(slot_at(SIDE_PADDING + 50.0, 3, width), 0);
    assert_eq!(slot_at(SIDE_PADDING + 150.0, 3, width), 1);
    assert_eq!(slot_at(9999.0, 3, width), 2);
    assert_eq!(slot_at(10.0, 0, width), 0);
}

#[test]
fn insertion_picks_the_nearest_gap() {
    let width = 100.0;
    assert_eq!(insertion_at(-30.0, 3, width), 0);
    assert_eq!(insertion_at(SIDE_PADDING + 40.0, 3, width), 0);
    assert_eq!(insertion_at(SIDE_PADDING + 60.0, 3, width), 1);
    assert_eq!(insertion_at(9999.0, 3, width), 3);
}

#[test]
fn tabs_after_an_incoming_gap_shift_one_slot() {
    assert_eq!(slot_of(0, None), 0);
    assert_eq!(slot_of(1, Some(2)), 1);
    assert_eq!(slot_of(2, Some(2)), 3);
    assert_eq!(slot_of(0, Some(0)), 1);
}

#[test]
fn a_dragged_tab_follows_the_pointer_between_the_end_slots() {
    let width = 100.0;
    let first = tab_left(0, width);
    let last = tab_left(2, width);
    // The pointer grabbed the tab 30 in: it sits 30 left of the pointer.
    assert_eq!(dragged_left(first + 130.0, 30.0, 3, width), first + 100.0);
    assert_eq!(dragged_left(-500.0, 30.0, 3, width), first);
    assert_eq!(dragged_left(9999.0, 30.0, 3, width), last);
    assert_eq!(dragged_left(50.0, 0.0, 0, width), first);
}

#[test]
fn a_dragged_tab_swaps_once_its_centre_crosses_a_neighbour() {
    let width = 100.0;
    let step = width + TAB_GAP;
    // Slot 0, dragged right: still slot 0 until its centre passes the slot-0/1 boundary.
    let left = tab_left(0, width);
    assert_eq!(dragged_slot(left, 3, width), 0);
    assert_eq!(dragged_slot(left + step * 0.49, 3, width), 0);
    assert_eq!(dragged_slot(left + step * 0.51, 3, width), 1);
    assert_eq!(dragged_slot(left + step * 5.0, 3, width), 2);
}

#[test]
fn tearing_off_needs_a_clear_exit_from_the_strip() {
    let strip = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, HEIGHT));
    assert!(!is_outside_tear_off_zone(pos2(400.0, 20.0), strip));
    assert!(!is_outside_tear_off_zone(pos2(400.0, HEIGHT + 10.0), strip));
    assert!(is_outside_tear_off_zone(pos2(400.0, HEIGHT * 2.5), strip));
    assert!(is_outside_tear_off_zone(pos2(-HEIGHT - 1.0, 10.0), strip));
}
