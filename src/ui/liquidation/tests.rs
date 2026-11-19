use super::*;
use crate::model::liquidation::{Exchange, Tape};
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

#[test]
fn the_panel_draws_empty_filtered_and_full() {
    let mut state = LiquidationsState::default();
    let full = busy_snapshot(MinSize::Any);
    let filtered = Snapshot {
        latest: Vec::new(),
        ..full.clone()
    };
    for snapshot in [Snapshot::default(), filtered, full] {
        egui::__run_test_ui(|ui| {
            assert_eq!(show(ui, &mut state, &snapshot), None);
        });
    }
}

#[test]
#[ignore = "writes pictures"]
fn gallery_of_the_panel() {
    use crate::ui::theme::{self, AppTheme, RestoreDefaultTheme};
    let _restore = RestoreDefaultTheme;
    let cases = [
        ("busy", busy_snapshot(MinSize::Any), MinSize::Any),
        (
            "big",
            busy_snapshot(MinSize::HundredThousand),
            MinSize::HundredThousand,
        ),
        ("empty", Snapshot::default(), MinSize::Any),
        (
            "filtered",
            Snapshot {
                latest: Vec::new(),
                ..busy_snapshot(MinSize::Any)
            },
            MinSize::Million,
        ),
    ];
    for (theme_name, app_theme) in [("dark", AppTheme::Dark), ("light", AppTheme::Light)] {
        for (name, snapshot, minimum) in &cases {
            for (size_name, width) in [("default", 340.0), ("narrow", 260.0)] {
                let mut screen = Screen::new(vec2(width, 640.0));
                theme::apply(&screen.ctx, app_theme);
                let mut state = LiquidationsState { minimum: *minimum };
                let mut last = None;
                for _ in 0..4 {
                    last = Some(screen.run(Vec::new(), |ui| {
                        ui.painter()
                            .rect_filled(ui.max_rect(), 0.0, theme::chrome());
                        show(ui, &mut state, snapshot);
                    }));
                }
                screen.save(
                    &format!("liquidations-{name}-{size_name}-{theme_name}"),
                    last.expect("a frame"),
                );
            }
        }
    }
}
