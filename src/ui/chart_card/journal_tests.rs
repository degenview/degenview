//! The journal's trades on the chart: drawn where the fills were, left out of snapshots unless
//! trading is included, hovered and clicked on the live chart, never moving the price scale.

use std::sync::Arc;

use super::canvas_tests::{HOUR, Harness, SIZE};
use super::*;
use crate::model::ChartCanvas;
use crate::model::journal::filters::OutcomeFilter;
use crate::model::journal::marks::{MarkFill, TradeMark};
use crate::model::journal::model::{Direction, Side};
use crate::model::snapshot::{SnapshotDimensions, SnapshotScale};
use eframe::egui::epaint::ClippedShape;
use rust_decimal::Decimal;

/// A winning long entered inside bar 250 and left inside bar 262 (the bars are an hour each).
fn winner(candles: &[Kline]) -> TradeMark {
    let (entry_bar, exit_bar) = (&candles[250], &candles[262]);
    let (entry_ts, exit_ts) = (
        entry_bar.open_time + 20 * 60_000,
        exit_bar.open_time + 40 * 60_000,
    );
    let (entry, exit) = (entry_bar.close, exit_bar.close + 1.0);
    TradeMark {
        key: "t-win".into(),
        account_uid: "a".into(),
        direction: Direction::Long,
        outcome: OutcomeFilter::Win,
        open_ts_ms: entry_ts,
        close_ts_ms: Some(exit_ts),
        fills: vec![
            MarkFill {
                ts_ms: entry_ts,
                price: entry,
                quantity: 1.0,
                side: Side::Buy,
                is_entry: true,
            },
            MarkFill {
                ts_ms: exit_ts,
                price: exit,
                quantity: 1.0,
                side: Side::Sell,
                is_entry: false,
            },
        ],
        avg_entry: entry,
        avg_exit: Some(exit),
        label: Some("+12.34".into()),
        currency: Some("USD".into()),
        quantity: Decimal::ONE,
        display_name: "BTCUSDT".into(),
    }
}

fn with_marks(harness: &mut Harness) {
    let mark = winner(&harness.candles);
    harness.card.journal.marks = Some(Arc::new(vec![mark]));
}

fn settle(harness: &mut Harness) -> Vec<ClippedShape> {
    harness.card.set_scale_menu(false);
    let mut last = harness.frame().0;
    for _ in 0..4 {
        last = harness.frame().0;
    }
    last
}

/// The chip's text, if the chart painted it.
fn chip(shapes: &[ClippedShape]) -> Option<Pos2> {
    shapes.iter().find_map(|clipped| match &clipped.shape {
        Shape::Text(text) if text.galley.text() == "+12.34" => Some(text.pos),
        _ => None,
    })
}

/// The centre of the filled triangles and diamonds the trade was drawn with.
fn glyph_centres(shapes: &[ClippedShape]) -> Vec<Pos2> {
    shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            Shape::Path(path) if path.closed && matches!(path.points.len(), 3 | 4) => {
                let sum = path
                    .points
                    .iter()
                    .fold(Vec2::ZERO, |sum, point| sum + point.to_vec2());
                Some((sum / path.points.len() as f32).to_pos2())
            }
            _ => None,
        })
        .collect()
}

#[test]
fn a_trade_is_drawn_at_its_fills_and_the_switch_turns_it_off() {
    let mut off = Harness::new(ChartCanvas::default());
    let plain = settle(&mut off);
    assert!(chip(&plain).is_none());

    let mut harness = Harness::new(ChartCanvas::default());
    with_marks(&mut harness);
    let drawn = settle(&mut harness);
    let chip_at = chip(&drawn).expect("the result chip");
    // Entry triangle and exit diamond, left to right, with the exit under the chip.
    let glyphs = glyph_centres(&drawn);
    let baseline = glyph_centres(&plain);
    let own: Vec<Pos2> = glyphs
        .into_iter()
        .filter(|g| !baseline.contains(g))
        .collect();
    assert_eq!(own.len(), 2, "{own:?}");
    assert!(own[0].x < own[1].x);
    // Twelve bars apart on screen.
    assert!(own[1].x - own[0].x > 20.0);
    assert!((chip_at.x - own[1].x).abs() < 60.0);

    harness.card.appearance_mut().canvas.show_journal_trades = false;
    let hidden = settle(&mut harness);
    assert!(chip(&hidden).is_none());
}

#[test]
fn the_trade_follows_the_chart_when_it_is_panned() {
    let mut harness = Harness::new(ChartCanvas::default());
    with_marks(&mut harness);
    let before = glyph_centres(&settle(&mut harness));
    harness
        .card
        .scroll_to_time(200 * HOUR, &harness.candles.clone());
    let after = settle(&mut harness);
    // Scrolled back by a good way: the trade is no longer on the plot, so its glyphs are gone.
    assert!(chip(&after).is_none() || glyph_centres(&after) != before);
}

#[test]
fn a_snapshot_shows_trades_only_when_trading_is_included() {
    let mut harness = Harness::new(ChartCanvas::default());
    with_marks(&mut harness);
    let live = settle(&mut harness);
    let rect = harness_rect(&mut harness);
    let without = snapshot_shapes(&harness, rect, false);
    let with = snapshot_shapes(&harness, rect, true);
    assert!(chip(&live).is_some());
    assert!(
        chip(&without).is_none(),
        "trading is not included by default"
    );
    assert!(chip(&with).is_some());
    assert!(
        chip(&with).is_some_and(|at| chip(&live).is_some_and(|live| (live - at).length() < 1.0))
    );
}

fn harness_rect(harness: &mut Harness) -> Rect {
    harness.frame().1.rect.expect("the chart's rectangle")
}

fn input(harness: &Harness, include_trading: bool) -> SnapshotInput<'_> {
    SnapshotInput {
        symbol: "BTCUSDT",
        source: DataSource::Binance,
        candles: &harness.candles,
        scale_mode: PriceScaleMode::Regular,
        indicators: &harness.indicators,
        drawings: &harness.drawings,
        open_notes: &[],
        images: None,
        hide_all_drawings: false,
        replay_active: harness.replay_active,
        include_trading,
    }
}

fn snapshot_shapes(harness: &Harness, rect: Rect, include_trading: bool) -> Vec<ClippedShape> {
    let ctx = egui::Context::default();
    let raw = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, SIZE)),
        ..Default::default()
    };
    let mut output = ctx.run_ui(raw, |ui| {
        harness
            .card
            .paint_snapshot(&ui.painter_at(rect), rect, &input(harness, include_trading))
            .expect("a chart to paint");
    });
    output.textures_delta.clear();
    std::mem::take(&mut output.shapes)
}

/// Pictures of a chart with several trades: `DEGENVIEW_GALLERY_DIR=<dir> cargo test --lib
/// chart_card::journal_tests::gallery -- --ignored`.
#[test]
#[ignore = "writes a picture"]
fn gallery_of_trades_on_a_chart() {
    let Some(dir) = std::env::var_os("DEGENVIEW_GALLERY_DIR").map(std::path::PathBuf::from) else {
        return;
    };
    for light in [false, true] {
        let _restore = crate::ui::theme::RestoreDefaultTheme;
        let mut harness = Harness::new(ChartCanvas::default());
        harness.ctx.set_theme(if light {
            egui::ThemePreference::Light
        } else {
            egui::ThemePreference::Dark
        });
        let candles = harness.candles.clone();
        let make =
            |key: &str, from: usize, to: usize, exit_offset: f64, long: bool, label: &str| {
                let mut mark = winner(&candles);
                let (a, b) = (&candles[from], &candles[to]);
                mark.key = key.into();
                mark.direction = if long {
                    Direction::Long
                } else {
                    Direction::Short
                };
                mark.open_ts_ms = a.open_time + 15 * 60_000;
                mark.close_ts_ms = Some(b.open_time + 30 * 60_000);
                let (entry, exit) = (a.close, b.close + exit_offset);
                let (open_side, close_side) = if long {
                    (Side::Buy, Side::Sell)
                } else {
                    (Side::Sell, Side::Buy)
                };
                mark.fills = vec![
                    MarkFill {
                        ts_ms: mark.open_ts_ms,
                        price: entry,
                        quantity: 1.0,
                        side: open_side,
                        is_entry: true,
                    },
                    MarkFill {
                        ts_ms: mark.close_ts_ms.unwrap(),
                        price: exit,
                        quantity: 1.0,
                        side: close_side,
                        is_entry: false,
                    },
                ];
                mark.avg_entry = entry;
                mark.avg_exit = Some(exit);
                let won = if long { exit > entry } else { exit < entry };
                mark.outcome = if won {
                    OutcomeFilter::Win
                } else {
                    OutcomeFilter::Loss
                };
                mark.label = Some(label.into());
                mark
            };
        let mut open = make("open", 285, 285, 0.0, true, "");
        open.outcome = OutcomeFilter::Open;
        open.close_ts_ms = None;
        open.fills.truncate(1);
        open.avg_exit = None;
        open.label = None;
        harness.card.journal.marks = Some(Arc::new(vec![
            make("a", 215, 224, 6.0, true, "+112.88"),
            make("b", 228, 240, 9.0, false, "-95.69"),
            make("c", 246, 262, 14.0, true, "+240.10"),
            make("d", 266, 276, -4.0, true, "-41.20"),
            open,
        ]));
        harness.card.set_scale_menu(false);
        for _ in 0..5 {
            harness.frame();
        }
        let size = harness.card.snapshot_size();
        let dimensions =
            SnapshotDimensions::fit(size.x, size.y, SnapshotScale::X2).expect("a size");
        let scene = harness
            .card
            .snapshot_scene(
                &input(&harness, true),
                dimensions,
                crate::ui::theme::chrome(),
            )
            .expect("a chart to draw");
        let image = crate::service::snapshot_raster::rasterize(&scene).expect("drawn");
        let png = crate::service::snapshot_export::encode_png(&image).expect("encoded");
        let name = if light { "light" } else { "dark" };
        std::fs::write(dir.join(format!("journal-chart-markers-{name}.png")), png)
            .expect("written");
    }
}

#[test]
fn clicking_a_trade_asks_for_it_and_clicking_elsewhere_does_not() {
    let mut harness = Harness::new(ChartCanvas::default());
    with_marks(&mut harness);
    let drawn = settle(&mut harness);
    let mut centres = glyph_centres(&drawn);
    centres.sort_by(|a, b| a.x.total_cmp(&b.x));
    let entry = centres[centres.len() - 2];

    // A click on the plot away from the trade opens nothing.
    harness.pointer = Some(Pos2::new(40.0, 40.0));
    harness.extra_events = click(Pos2::new(40.0, 40.0));
    harness.frame();
    assert_eq!(harness.card.journal.open, None);

    harness.pointer = Some(entry);
    harness.extra_events = click(entry);
    harness.frame();
    assert_eq!(harness.card.journal.open.as_deref(), Some("t-win"));
}

fn click(at: Pos2) -> Vec<egui::Event> {
    [true, false]
        .into_iter()
        .map(|pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        })
        .collect()
}

#[test]
fn a_trade_that_has_not_happened_yet_in_a_replay_is_not_shown() {
    let mut harness = Harness::new(ChartCanvas::default());
    with_marks(&mut harness);
    // The chart ends at bar 200, before the trade.
    harness.candles.truncate(200);
    let drawn = settle(&mut harness);
    assert!(chip(&drawn).is_none());
}

#[test]
fn the_trade_never_changes_the_price_scale() {
    let mut plain = Harness::new(ChartCanvas::default());
    let before = settle(&mut plain);
    let mut harness = Harness::new(ChartCanvas::default());
    let mut mark = winner(&harness.candles);
    // Absurdly far off the price range.
    mark.fills[1].price = 1_000_000.0;
    mark.avg_exit = Some(1_000_000.0);
    harness.card.journal.marks = Some(Arc::new(vec![mark]));
    let after = settle(&mut harness);
    let ticks = |shapes: &[ClippedShape]| -> Vec<String> {
        shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                Shape::Text(text) if text.pos.x > SIZE.x - 90.0 => {
                    Some(text.galley.text().to_owned())
                }
                _ => None,
            })
            .collect()
    };
    assert_eq!(ticks(&before), ticks(&after));
}
