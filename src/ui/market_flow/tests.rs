use super::*;
use crate::model::liquidation::{Exchange, Tape};
use crate::model::taker_flow::{Flow, Taker, Trade, Venue};
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

#[test]
fn the_panel_draws_every_state_of_both_modes() {
    let mut state = MarketFlowState::default();
    let full = busy_snapshot(MinSize::Any);
    let filtered = Snapshot {
        latest: Vec::new(),
        ..full.clone()
    };
    let flow = busy_flow();
    let partial = partial_flow();
    let quiet = taker_flow::Snapshot::default();
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
    ];
    for body in &bodies {
        egui::__run_test_ui(|ui| {
            assert_eq!(show(ui, &mut state, body), None);
        });
    }
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
    ];
    for (theme_name, app_theme) in [("dark", AppTheme::Dark), ("light", AppTheme::Light)] {
        for (name, mode, minimum, body) in &cases {
            for (size_name, width) in [("default", 340.0), ("narrow", 280.0)] {
                let mut screen = Screen::new(vec2(width, 640.0));
                theme::apply(&screen.ctx, app_theme);
                let mut state = MarketFlowState {
                    mode: *mode,
                    minimum: *minimum,
                };
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
