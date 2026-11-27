use super::*;
use crate::model::depth::{
    BookEvent, Delta, DepthRecord, DepthSnapshot, DiffStream, Event, Levels, Price,
    Snapshot as BookSnapshot, Trade as DepthTrade, View,
};
use crate::model::exchange::{Exchange, Venue};
use crate::model::liquidation::Tape;
use crate::model::taker_flow::{Flow, Taker, Trade};
use crate::ui::render_tests::Screen;

const NOW: i64 = 1_795_000_000_000;

fn liquidation(side: Side, symbol: &str, notional: f64, age_ms: i64) -> Liquidation {
    Liquidation {
        exchange: if symbol.len().is_multiple_of(2) {
            Exchange::Binance
        } else {
            Exchange::Bybit
        },
        symbol: symbol.to_owned(),
        side,
        price: 100.0,
        quantity: notional / 100.0,
        time_ms: NOW - age_ms,
    }
}

fn busy_snapshot(minimum: MinSize) -> Snapshot {
    let mut tape = Tape::default();
    let symbols = ["BTCUSDT", "ETHUSDT", "SOLUSDT", "DOGEUSDT", "1000PEPEUSDT"];
    for index in 0..120_i64 {
        let side = if index % 3 == 0 {
            Side::Short
        } else {
            Side::Long
        };
        let notional = 400.0 * (1 + index % 17).pow(3) as f64;
        tape.push(
            liquidation(
                side,
                symbols[index as usize % symbols.len()],
                notional,
                (120 - index) * 20_000,
            ),
            NOW,
        );
    }
    tape.snapshot(NOW, minimum)
}

#[test]
fn amounts_are_short_and_keep_their_magnitude() {
    assert_eq!(usd(850.4), "$850");
    assert_eq!(usd(1_234.0), "$1.2K");
    assert_eq!(usd(34_600.0), "$35K");
    assert_eq!(usd(1_234_567.0), "$1.23M");
    assert_eq!(usd(2_500_000_000.0), "$2.50B");
    assert_eq!(usd(-5_000.0), "$5.0K");
    assert_eq!(usd(0.0), "$0");
}

#[test]
fn longs_read_red_and_shorts_green() {
    assert_eq!(side_color(Side::Long), theme::DOWN);
    assert_eq!(side_color(Side::Short), theme::UP);
}

/// Half an hour of trades on every venue, the buyers ahead early and the sellers late.
fn busy_flow() -> taker_flow::Snapshot {
    let mut flow = Flow::default();
    for venue in Venue::ALL {
        flow.link(venue, true, NOW - 3_600_000);
    }
    for second in 0..1_800_i64 {
        for (index, venue) in Venue::ALL.iter().enumerate() {
            let leaning_up = second > 900;
            let taker = if (second + index as i64) % 5 < if leaning_up { 3 } else { 2 } {
                Taker::Buy
            } else {
                Taker::Sell
            };
            flow.push(
                Trade {
                    venue: *venue,
                    notional: 40_000.0 + 25_000.0 * ((second * 7 + index as i64) % 11) as f64,
                    taker,
                    time_ms: NOW - second * 1_000,
                },
                NOW,
            );
        }
    }
    flow.snapshot(NOW)
}

/// A panel just opened: one venue heard a trade a few seconds after connecting, one never
/// connected, and an outage left a hole in the bars.
fn partial_flow() -> taker_flow::Snapshot {
    let mut flow = Flow::default();
    flow.link(Venue::ALL[0], true, NOW - 5 * 60_000);
    flow.link(Venue::ALL[0], false, NOW - 3 * 60_000);
    flow.link(Venue::ALL[0], true, NOW - 2 * 60_000);
    flow.link(Venue::ALL[1], true, NOW - 20_000);
    for (index, age) in [(0, 150_000), (0, 30_000), (1, 5_000)]
        .into_iter()
        .enumerate()
    {
        flow.push(
            Trade {
                venue: Venue::ALL[age.0],
                notional: 80_000.0 + 40_000.0 * index as f64,
                taker: if index == 1 { Taker::Buy } else { Taker::Sell },
                time_ms: NOW - age.1,
            },
            NOW,
        );
    }
    flow.snapshot(NOW)
}

fn price(text: &str) -> Price {
    Price::parse(text).unwrap()
}

fn levels(rows: &[(Price, f64)]) -> Vec<(Price, f64)> {
    rows.to_vec()
}

/// A BTC book with a cent tick, 1,500 levels a side with sizes that vary, 60 trades (a few big ones)
/// and some size stacked and pulled in the last seconds.
pub(in crate::ui::market_flow) fn live_book() -> DepthRecord {
    let mut record = DepthRecord::new(Box::new(DiffStream::default()));
    record.set_tick(price("0.01"));
    record.link(true);
    let sized = |index: i64, seed: i64| 0.05 + ((index * 37 + seed * 11) % 23) as f64 * 0.31;
    // Levels thin out away from the market: a cent apart at first, then a few dollars.
    let away = |index: i64| index * 1_000_000 + index * index * 40_000;
    let bids: Vec<_> = (0..1_500_i64)
        .map(|index| {
            (
                Price::from_units(5_817_151_000_000 - away(index)),
                sized(index, 1),
            )
        })
        .collect();
    let asks: Vec<_> = (0..1_500_i64)
        .map(|index| {
            (
                Price::from_units(5_817_152_000_000 + away(index)),
                sized(index, 5),
            )
        })
        .collect();
    record.apply(
        Event::Book(BookEvent::Snapshot(BookSnapshot {
            id: 100,
            levels: Levels { bids, asks },
        })),
        NOW - 60_000,
    );
    for second in 0..60_i64 {
        let buy = second % 3 != 0;
        let offset = (second * 7) % 9 - 4;
        let at = if buy {
            5_817_152_000_000
        } else {
            5_817_151_000_000
        } + offset * 1_000_000;
        let size = if second % 13 == 5 {
            2.4
        } else {
            0.01 + (second % 7) as f64 * 0.12
        };
        record.apply(
            Event::Trade(DepthTrade {
                time_ms: NOW - 60_000 + second * 1_000,
                price: Price::from_units(at),
                size,
                taker: if buy {
                    crate::model::taker_flow::Taker::Buy
                } else {
                    crate::model::taker_flow::Taker::Sell
                },
            }),
            NOW - 60_000 + second * 1_000,
        );
    }
    // Two seconds ago: bid size stacked at three prices and an offer pulled at two.
    let change = Delta {
        previous: None,
        first: 101,
        last: 101,
        levels: Levels {
            bids: levels(&[
                (price("58171.51"), 9.0),
                (price("58171.50"), 6.5),
                (price("58171.46"), 12.0),
                (price("58160.00"), 140.0),
            ]),
            asks: levels(&[(price("58171.53"), 0.0), (price("58171.55"), 0.0)]),
        },
    };
    record.apply(Event::Book(BookEvent::Delta(change)), NOW - 2_000);
    record
}

pub(in crate::ui::market_flow) fn live_depth(view: View) -> DepthSnapshot {
    live_book().snapshot(view, NOW)
}

/// What the panel asks for before it has been drawn once.
pub(in crate::ui::market_flow) fn first_view() -> View {
    BookViewState::default().view()
}

/// Depth that has no book to show, for a link that is `connecting`, dropped or unlisted.
pub(in crate::ui::market_flow) fn waiting_depth(link: crate::model::depth::Link) -> DepthSnapshot {
    DepthSnapshot::waiting(link, Some(price("0.01")))
}

pub(in crate::ui::market_flow) fn feed(snapshot: &DepthSnapshot) -> BookFeed<'_> {
    BookFeed {
        venue: Venue {
            exchange: Exchange::Binance,
            market: crate::model::exchange::Market::Spot,
        },
        symbol: "BTCUSDT",
        snapshot,
    }
}

/// The depth for `make`, asked for the rows the panel has room for in a screen of `size`: the
/// panel is drawn until what it measured is what it was given, as it is in the app.
fn settled_depth(
    size: egui::Vec2,
    state: &mut MarketFlowState,
    make: &dyn Fn(View) -> DepthSnapshot,
) -> DepthSnapshot {
    let mut screen = Screen::new(size);
    let mut depth = make(state.book.view());
    for _ in 0..3 {
        screen.run(Vec::new(), |ui| {
            let body = Body::Book(BookBody {
                feed: Some(feed(&depth)),
            });
            show(ui, state, &body);
        });
        depth = make(state.book.view());
    }
    depth
}

#[test]
fn the_panel_draws_every_state_of_every_mode() {
    let mut state = MarketFlowState::default();
    let full = busy_snapshot(MinSize::Any);
    let filtered = Snapshot {
        latest: Vec::new(),
        ..full.clone()
    };
    let flow = busy_flow();
    let partial = partial_flow();
    let quiet = taker_flow::Snapshot::default();
    let live = live_depth(first_view());
    let grouped = live_depth(View {
        group: Some(price("10")),
        centre: Some(price("58300")),
        ..first_view()
    });
    let connecting = waiting_depth(crate::model::depth::Link::Connecting);
    let reconnecting = waiting_depth(crate::model::depth::Link::Reconnecting);
    let unlisted = waiting_depth(crate::model::depth::Link::Unlisted);
    let bodies = [
        Body::Liquidations(&Snapshot::default()),
        Body::Liquidations(&filtered),
        Body::Liquidations(&full),
        Body::TakerFlow(FlowBody {
            asset: None,
            snapshot: None,
        }),
        Body::TakerFlow(FlowBody {
            asset: Some("BTC"),
            snapshot: Some(&quiet),
        }),
        Body::TakerFlow(FlowBody {
            asset: Some("BTC"),
            snapshot: Some(&flow),
        }),
        Body::TakerFlow(FlowBody {
            asset: Some("BTC"),
            snapshot: Some(&partial),
        }),
        Body::Book(BookBody { feed: None }),
        Body::Book(BookBody {
            feed: Some(feed(&live)),
        }),
        Body::Book(BookBody {
            feed: Some(feed(&grouped)),
        }),
        Body::Book(BookBody {
            feed: Some(feed(&connecting)),
        }),
        Body::Book(BookBody {
            feed: Some(feed(&reconnecting)),
        }),
        Body::Book(BookBody {
            feed: Some(feed(&unlisted)),
        }),
    ];
    for body in &bodies {
        egui::__run_test_ui(|ui| {
            assert_eq!(show(ui, &mut state, body), None);
        });
    }
}

/// Every state of every mode, drawn in the pseudo-locale at the panel's narrowest: no word is left
/// in English, and none is cut short.
#[test]
fn every_word_of_the_panel_is_translated_and_fits_in_the_pseudo_locale() {
    use crate::i18n::{Locale, with_locale};
    let full = busy_snapshot(MinSize::Any);
    let filtered = Snapshot {
        latest: Vec::new(),
        ..full.clone()
    };
    let empty = Snapshot::default();
    let flow = busy_flow();
    let partial = partial_flow();
    let quiet = taker_flow::Snapshot::default();
    let connecting = waiting_depth(crate::model::depth::Link::Connecting);
    let reconnecting = waiting_depth(crate::model::depth::Link::Reconnecting);
    let unlisted = waiting_depth(crate::model::depth::Link::Unlisted);
    let live_view = settled_depth(
        vec2(280.0, 640.0),
        &mut MarketFlowState::default(),
        &live_depth,
    );
    let none = FlowBody {
        asset: None,
        snapshot: None,
    };
    let book = |snapshot| {
        Body::Book(BookBody {
            feed: Some(feed(snapshot)),
        })
    };
    let cases = [
        (
            "liquidations",
            Mode::Liquidations,
            Body::Liquidations(&full),
        ),
        (
            "filtered",
            Mode::Liquidations,
            Body::Liquidations(&filtered),
        ),
        ("empty", Mode::Liquidations, Body::Liquidations(&empty)),
        ("flow none", Mode::TakerFlow, Body::TakerFlow(none)),
        (
            "flow waiting",
            Mode::TakerFlow,
            Body::TakerFlow(FlowBody {
                asset: Some("BTC"),
                snapshot: Some(&quiet),
            }),
        ),
        (
            "flow",
            Mode::TakerFlow,
            Body::TakerFlow(FlowBody {
                asset: Some("BTC"),
                snapshot: Some(&flow),
            }),
        ),
        (
            "flow partial",
            Mode::TakerFlow,
            Body::TakerFlow(FlowBody {
                asset: Some("BTC"),
                snapshot: Some(&partial),
            }),
        ),
        ("book none", Mode::Book, Body::Book(BookBody { feed: None })),
        ("book live", Mode::Book, book(&live_view)),
        ("book connecting", Mode::Book, book(&connecting)),
        ("book reconnecting", Mode::Book, book(&reconnecting)),
        ("book unlisted", Mode::Book, book(&unlisted)),
    ];
    with_locale(Locale::PSEUDO, || {
        for (name, mode, body) in &cases {
            let mut screen = Screen::new(vec2(280.0, 640.0));
            let mut state = MarketFlowState {
                mode: *mode,
                ..MarketFlowState::default()
            };
            let mut last = None;
            for _ in 0..3 {
                last = Some(screen.run(Vec::new(), |ui| {
                    show(ui, &mut state, body);
                }));
            }
            let frame = last.expect("a frame");
            assert_eq!(
                Screen::truncated_text(&frame),
                Vec::<String>::new(),
                "{name}: text is cut"
            );
            let words = Screen::words(&frame);
            assert!(words.len() > 3, "{name}: nothing was drawn");
            assert_eq!(
                Screen::untranslated(&words, &[]),
                Vec::<&str>::new(),
                "{name}: text is not translated"
            );
        }
    });
}

#[test]
fn the_labels_of_the_panel_follow_the_language() {
    use crate::i18n::{Locale, with_locale};
    use crate::model::exchange::Venue;
    assert_eq!(side_name(Side::Long), "Long");
    assert_eq!(MinSize::HundredThousand.label(), "$100K and up");
    assert_eq!(Venue::ALL[1].label(), "Binance perp");
    with_locale(Locale::PSEUDO, || {
        assert_ne!(side_name(Side::Short), "Short");
        assert_ne!(MinSize::Any.label(), "Any size");
        assert_ne!(Venue::ALL[2].label(), "Bybit spot");
    });
}

#[test]
#[ignore = "writes pictures"]
fn gallery_of_the_panel() {
    use crate::ui::theme::{self, AppTheme, RestoreDefaultTheme};
    let _restore = RestoreDefaultTheme;
    let busy = busy_snapshot(MinSize::Any);
    let big = busy_snapshot(MinSize::HundredThousand);
    let filtered = Snapshot {
        latest: Vec::new(),
        ..busy.clone()
    };
    let empty = Snapshot::default();
    let flow = busy_flow();
    let partial = partial_flow();
    let quiet = taker_flow::Snapshot::default();
    // Placeholders: the pictures of these two are of depth asked for the rows that fit.
    let live = live_depth(first_view());
    let fine = live_depth(first_view());
    let make_live = |view: View| live_depth(view);
    let make_fine = |view: View| {
        live_depth(View {
            group: Some(price("0.5")),
            ..view
        })
    };
    let connecting = waiting_depth(crate::model::depth::Link::Connecting);
    let reconnecting = waiting_depth(crate::model::depth::Link::Reconnecting);
    let unlisted = waiting_depth(crate::model::depth::Link::Unlisted);
    let cases: Vec<(&str, Mode, MinSize, Body<'_>)> = vec![
        (
            "busy",
            Mode::Liquidations,
            MinSize::Any,
            Body::Liquidations(&busy),
        ),
        (
            "big",
            Mode::Liquidations,
            MinSize::HundredThousand,
            Body::Liquidations(&big),
        ),
        (
            "empty",
            Mode::Liquidations,
            MinSize::Any,
            Body::Liquidations(&empty),
        ),
        (
            "filtered",
            Mode::Liquidations,
            MinSize::Million,
            Body::Liquidations(&filtered),
        ),
        (
            "flow",
            Mode::TakerFlow,
            MinSize::Any,
            Body::TakerFlow(FlowBody {
                asset: Some("BTC"),
                snapshot: Some(&flow),
            }),
        ),
        (
            "flow-partial",
            Mode::TakerFlow,
            MinSize::Any,
            Body::TakerFlow(FlowBody {
                asset: Some("BTC"),
                snapshot: Some(&partial),
            }),
        ),
        (
            "flow-waiting",
            Mode::TakerFlow,
            MinSize::Any,
            Body::TakerFlow(FlowBody {
                asset: Some("PEPE"),
                snapshot: Some(&quiet),
            }),
        ),
        (
            "flow-none",
            Mode::TakerFlow,
            MinSize::Any,
            Body::TakerFlow(FlowBody {
                asset: None,
                snapshot: None,
            }),
        ),
        (
            "book-live",
            Mode::Book,
            MinSize::Any,
            Body::Book(BookBody {
                feed: Some(feed(&live)),
            }),
        ),
        (
            "book-fine",
            Mode::Book,
            MinSize::Any,
            Body::Book(BookBody {
                feed: Some(feed(&fine)),
            }),
        ),
        (
            "book-connecting",
            Mode::Book,
            MinSize::Any,
            Body::Book(BookBody {
                feed: Some(feed(&connecting)),
            }),
        ),
        (
            "book-reconnecting",
            Mode::Book,
            MinSize::Any,
            Body::Book(BookBody {
                feed: Some(feed(&reconnecting)),
            }),
        ),
        (
            "book-unlisted",
            Mode::Book,
            MinSize::Any,
            Body::Book(BookBody {
                feed: Some(feed(&unlisted)),
            }),
        ),
        (
            "book-unsupported",
            Mode::Book,
            MinSize::Any,
            Body::Book(BookBody { feed: None }),
        ),
    ];
    for (theme_name, app_theme) in [("dark", AppTheme::Dark), ("light", AppTheme::Light)] {
        for (name, mode, minimum, body) in &cases {
            for (size_name, width) in [("default", 340.0), ("narrow", 280.0)] {
                let mut screen = Screen::new(vec2(width, 640.0));
                theme::apply(&screen.ctx, app_theme);
                let mut state = MarketFlowState {
                    mode: *mode,
                    minimum: *minimum,
                    ..MarketFlowState::default()
                };
                // One picture shows the toolbar of a ladder that was scrolled away from the market.
                if *name == "book-fine" {
                    state.book.centre = Some(price("58173"));
                }
                let make: Option<&dyn Fn(View) -> DepthSnapshot> = match *name {
                    "book-live" => Some(&make_live),
                    "book-fine" => Some(&make_fine),
                    _ => None,
                };
                let settled = make.map(|make| settled_depth(vec2(width, 640.0), &mut state, make));
                let settled_body = settled.as_ref().map(|depth| {
                    Body::Book(BookBody {
                        feed: Some(feed(depth)),
                    })
                });
                let body = settled_body.as_ref().unwrap_or(body);
                let mut last = None;
                for _ in 0..4 {
                    last = Some(screen.run(Vec::new(), |ui| {
                        ui.painter()
                            .rect_filled(ui.max_rect(), 0.0, theme::chrome());
                        show(ui, &mut state, body);
                    }));
                }
                screen.save(
                    &format!("panel-{name}-{size_name}-{theme_name}"),
                    last.expect("a frame"),
                );
            }
        }
    }
}

/// The panel with the group menu open, to see that the menu is as wide as its rows and no wider.
#[test]
#[ignore = "writes pictures"]
fn gallery_of_the_book_menus() {
    use crate::ui::theme::{self, AppTheme, RestoreDefaultTheme};
    use eframe::egui::{Event, Modifiers, PointerButton};
    let _restore = RestoreDefaultTheme;
    let mut asked = MarketFlowState::default();
    let live = settled_depth(vec2(280.0, 640.0), &mut asked, &live_depth);
    let body = Body::Book(BookBody {
        feed: Some(feed(&live)),
    });
    // Where the pickers sit in the toolbar, at the panel's narrowest.
    for (menu, at) in [("group", pos2(90.0, 64.0)), ("big", pos2(176.0, 64.0))] {
        for (theme_name, app_theme) in [("dark", AppTheme::Dark), ("light", AppTheme::Light)] {
            let mut screen = Screen::new(vec2(280.0, 640.0));
            theme::apply(&screen.ctx, app_theme);
            let mut state = MarketFlowState {
                mode: Mode::Book,
                ..MarketFlowState::default()
            };
            let mut draw = |screen: &mut Screen, events: Vec<Event>| {
                screen.run(events, |ui| {
                    ui.painter()
                        .rect_filled(ui.max_rect(), 0.0, theme::chrome());
                    show(ui, &mut state, &body);
                })
            };
            for _ in 0..3 {
                draw(&mut screen, Vec::new());
            }
            let click = |pressed| Event::PointerButton {
                pos: at,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            };
            draw(&mut screen, vec![Event::PointerMoved(at)]);
            draw(&mut screen, vec![click(true)]);
            draw(&mut screen, vec![click(false)]);
            let mut last = None;
            for _ in 0..3 {
                last = Some(draw(&mut screen, Vec::new()));
            }
            screen.save(
                &format!("panel-book-menu-{menu}-{theme_name}"),
                last.expect("a frame"),
            );
        }
    }
}

/// The close button never lies over the last mode tab: words longer than English's give it a row
/// of its own.
#[test]
fn the_close_button_gets_its_own_row_when_the_mode_tabs_do_not_fit_beside_it() {
    use crate::i18n::{Locale, with_locale};
    let tabs = || {
        [
            kit::Segment::new(Mode::Liquidations, tr!("Liquidations")),
            kit::Segment::new(Mode::TakerFlow, tr!("Taker flow")),
            kit::Segment::new(Mode::Book, tr!("Book")),
        ]
    };
    let mut screen = Screen::new(vec2(340.0, 200.0));
    for _ in 0..3 {
        screen.run(Vec::new(), |ui| {
            let english = kit::segments_width(ui, &tabs());
            let pseudo = with_locale(Locale::PSEUDO, || kit::segments_width(ui, &tabs()));
            assert!(pseudo > english, "{english} {pseudo}");
            // A panel the English tabs share with the button, and the pseudo-locale's do not.
            let width = MARGIN + english + CLOSE_ROOM + 20.0;
            assert!(tabs_fit_beside_close(english, width));
            assert!(!tabs_fit_beside_close(pseudo, width));
        });
    }
}
