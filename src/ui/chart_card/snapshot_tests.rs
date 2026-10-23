#![allow(clippy::field_reassign_with_default)]
//! The snapshot against the live chart: the same frame, minus everything a pointer can reach.

use super::canvas_tests::{HOUR, Harness, SIZE, wait_for_run, with_panes};
use super::*;
use crate::model::snapshot::{SnapshotDimensions, SnapshotScale};
use crate::model::{ChartCanvas, Drawing, Tool};
use eframe::egui::ColorImage;
use eframe::egui::epaint::ClippedShape;

fn input(harness: &Harness, include_trading: bool) -> SnapshotInput<'_> {
    SnapshotInput {
        symbol: "BTCUSDT",
        source: DataSource::Binance,
        candles: &harness.candles,
        log_scale: false,
        indicators: &harness.indicators,
        drawings: &harness.drawings,
        open_notes: &[],
        images: None,
        hide_all_drawings: false,
        replay_active: harness.replay_active,
        include_trading,
    }
}

/// What the snapshot paints into `rect` of a plain context the size of the live one.
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

/// A shape written out. Text is its words, place, size and colour: where each glyph landed in
/// the font atlas depends on what a context drew before, not on what it drew.
fn describe(shape: &Shape) -> String {
    match shape {
        Shape::Text(text) => format!(
            "Text {:?} at {:?} {:?}",
            text.galley.text(),
            text.pos,
            text.galley
                .job
                .sections
                .iter()
                .map(|section| (section.format.font_id.clone(), section.format.color))
                .collect::<Vec<_>>()
        ),
        Shape::Vec(shapes) => shapes.iter().map(describe).collect::<Vec<_>>().join("; "),
        other => format!("{other:?}"),
    }
}

fn dumped(shapes: &[ClippedShape]) -> Vec<String> {
    shapes
        .iter()
        .map(|clipped| format!("{:?} {}", clipped.clip_rect, describe(&clipped.shape)))
        .collect()
}

/// A shape in a few words, for a failure to be read.
fn gist(dump: &str) -> String {
    dump.chars().take(160).collect()
}

/// The live chart's last frame with nothing pointing at it, and its plot's rectangle.
fn settled(harness: &mut Harness) -> (Vec<ClippedShape>, Rect) {
    let mut last = harness.frame();
    for _ in 0..4 {
        last = harness.frame();
    }
    (last.0, last.1.rect.expect("the chart's rectangle"))
}

/// What the live chart painted that the snapshot did not, and the other way round.
fn differences(live: &[ClippedShape], snapshot: &[ClippedShape]) -> (Vec<String>, Vec<String>) {
    let (mut live, mut snapshot) = (dumped(live), dumped(snapshot));
    live.retain(
        |shape| match snapshot.iter().position(|other| other == shape) {
            Some(index) => {
                snapshot.remove(index);
                false
            }
            None => true,
        },
    );
    (live, snapshot)
}

#[test]
fn a_plain_chart_snapshot_paints_what_the_live_chart_paints() {
    let mut harness = Harness::new(ChartCanvas::default());
    let (live, rect) = settled(&mut harness);
    let (only_live, only_snapshot) = differences(&live, &snapshot_shapes(&harness, rect, false));
    assert_eq!(
        only_snapshot,
        Vec::<String>::new(),
        "nothing the live chart lacks"
    );
    assert_eq!(
        only_live,
        Vec::<String>::new(),
        "nothing the snapshot lacks"
    );
}

#[test]
fn every_chart_type_snapshots_as_it_draws() {
    for chart_type in ChartType::ALL {
        let mut harness = Harness::new(ChartCanvas::default());
        harness.card.appearance_mut().style.chart_type = chart_type;
        let (live, rect) = settled(&mut harness);
        let (only_live, only_snapshot) =
            differences(&live, &snapshot_shapes(&harness, rect, false));
        assert!(
            only_snapshot.is_empty(),
            "{chart_type:?}: {}",
            only_snapshot.len()
        );
        assert!(only_live.is_empty(), "{chart_type:?}: {}", only_live.len());
    }
}

fn same_as_live(harness: &mut Harness, include_trading: bool) {
    let (live, rect) = settled(harness);
    let (only_live, only_snapshot) =
        differences(&live, &snapshot_shapes(harness, rect, include_trading));
    let gists = |dumps: &[String]| dumps.iter().map(|dump| gist(dump)).collect::<Vec<_>>();
    assert!(only_snapshot.is_empty(), "{:#?}", gists(&only_snapshot));
    assert!(only_live.is_empty(), "{:#?}", gists(&only_live));
}

fn everything_on() -> IndicatorConfig {
    IndicatorConfig {
        show_volume: true,
        show_rsi: true,
        show_ema: true,
        show_bollinger: true,
        show_trend_flips: true,
        ..IndicatorConfig::default()
    }
}

fn trend_line() -> Drawing {
    Drawing::build(
        Tool::TrendLine,
        1,
        &[
            crate::model::Anchor {
                time_ms: 250 * HOUR,
                price: 222.0,
            },
            crate::model::Anchor {
                time_ms: 270 * HOUR,
                price: 245.0,
            },
        ],
        Tool::TrendLine.default_style(),
    )
    .expect("builds")
}

#[test]
fn built_in_indicators_snapshot_as_they_draw() {
    let mut harness = Harness::new(ChartCanvas::default());
    harness.indicators = everything_on();
    same_as_live(&mut harness, false);
}

#[test]
fn drawings_snapshot_as_they_draw() {
    let mut harness = Harness::new(ChartCanvas::default());
    harness.drawings = vec![trend_line()];
    same_as_live(&mut harness, false);
}

#[test]
fn hidden_drawings_stay_hidden_and_visible_ones_are_drawn() {
    let mut harness = Harness::new(ChartCanvas::default());
    settled(&mut harness);
    let rect = Rect::from_min_size(Pos2::ZERO, SIZE);
    let sorted = |harness: &Harness, hide_all: bool| {
        let ctx = egui::Context::default();
        let raw = egui::RawInput {
            screen_rect: Some(rect),
            ..Default::default()
        };
        let mut input = input(harness, false);
        input.hide_all_drawings = hide_all;
        let mut out = ctx.run_ui(raw, |ui| {
            harness
                .card
                .paint_snapshot(&ui.painter_at(rect), rect, &input)
                .expect("a chart");
        });
        out.textures_delta.clear();
        let mut shapes = dumped(&std::mem::take(&mut out.shapes));
        shapes.sort();
        shapes
    };
    let none = sorted(&harness, false);
    harness.drawings = vec![trend_line()];
    let drawn = sorted(&harness, false);
    assert_ne!(none, drawn, "a visible drawing is in the picture");
    assert_eq!(none, sorted(&harness, true), "all drawings hidden");
    let mut hidden = trend_line();
    hidden.set_hidden(true);
    harness.drawings = vec![hidden];
    assert_eq!(none, sorted(&harness, false), "one hidden drawing");
}

const OVERLAY_VISUALS: &str = "//@version=6
indicator(\"Visuals\", overlay = true)
plot(ta.sma(close, 5), color = color.orange)
plotshape(close > open, style = shape.triangleup, location = location.belowbar, color = color.green)
bgcolor(close > open ? color.new(color.green, 92) : na)
if bar_index % 20 == 0
    label.new(bar_index, high, \"mark\")
    line.new(bar_index, low, bar_index + 5, high)
    box.new(bar_index, high, bar_index + 3, low)
var t = table.new(position.top_right, 1, 1)
table.cell(t, 0, 0, \"table text\")
";

#[test]
fn script_labels_lines_boxes_shapes_and_tables_snapshot_as_they_draw() {
    let mut harness = Harness::new(ChartCanvas::default());
    let id = harness
        .card
        .add_pine_instance(None, "Visuals", OVERLAY_VISUALS);
    wait_for_run(&mut harness, id);
    let output = harness.card.pine_visual_for_tests(id);
    assert!(
        output.labels > 0 && output.lines > 0 && output.boxes > 0 && output.tables > 0,
        "the script drew what the test needs: {output:?}"
    );
    same_as_live(&mut harness, false);
    // And the table's text really is in the image.
    let (_, rect) = settled(&mut harness);
    assert!(
        dumped(&snapshot_shapes(&harness, rect, false))
            .iter()
            .any(|shape| shape.contains("table text")),
        "the table is drawn"
    );
}

#[test]
fn script_panes_snapshot_as_they_draw() {
    same_as_live(&mut with_panes(ChartCanvas::default()), false);
}

// MARK: Account and alerts

fn with_account(harness: &mut Harness) {
    harness.card.paper.show_markers = true;
    let instrument =
        crate::model::paper::PaperInstrument::chart("BTCUSDT", "BTC/USDT", DataSource::Binance);
    harness.card.paper.positions = vec![(
        crate::model::paper::PaperPosition {
            account_id: 1,
            instrument,
            signed_quantity: 1.0,
            average_entry_price: 240.0,
            realized_gross_pnl: 0.0,
            commissions: 0.0,
            opened_at: 0,
            updated_at: 0,
        },
        12.5,
    )];
    harness
        .card
        .set_alert_lines(vec![crate::model::alert_line::AlertLine {
            id: 7,
            price: 230.0,
            paused: false,
            above: true,
            editable: true,
        }]);
}

#[test]
fn alerts_and_the_account_snapshot_as_the_chart_shows_them() {
    let mut harness = Harness::new(ChartCanvas::default());
    with_account(&mut harness);
    same_as_live(&mut harness, true);
}

#[test]
fn the_account_stays_out_of_a_snapshot_unless_asked_for() {
    let mut harness = Harness::new(ChartCanvas::default());
    with_account(&mut harness);
    let (live, rect) = settled(&mut harness);
    let (only_live, only_snapshot) = differences(&live, &snapshot_shapes(&harness, rect, false));
    assert!(only_snapshot.is_empty(), "{only_snapshot:?}");
    let missing = only_live.join("\n");
    assert!(
        missing.contains("Long") || missing.contains("LONG") || missing.contains("BUY"),
        "the position's pill is left out: {missing}"
    );
    assert!(
        !missing.contains("Alert"),
        "an alert is the chart's, not the account's, and stays"
    );
}

// MARK: What a pointer would add

#[test]
fn the_crosshair_a_hover_and_the_replay_badge_never_reach_a_snapshot() {
    let mut plain = Harness::new(ChartCanvas::default());
    let (_, rect) = settled(&mut plain);
    let mut pointed = Harness::new(ChartCanvas::default());
    pointed.crosshair = Some(CrosshairInfo {
        owner_id: 1,
        x_fraction: 0.4,
        price: 240.0,
    });
    pointed.pointer = Some(Pos2::new(300.0, 200.0));
    let (live, _) = settled(&mut pointed);
    let (only_live, _) = differences(&live, &snapshot_shapes(&pointed, rect, false));
    assert!(
        !only_live.is_empty(),
        "the live chart does draw a crosshair"
    );
    let shot = |harness: &Harness| {
        let mut shapes = dumped(&snapshot_shapes(harness, rect, false));
        shapes.sort();
        shapes
    };
    assert_eq!(shot(&plain), shot(&pointed));

    // A replay names itself in the watermark; its badge and pick marker are the live chart's.
    let mut canvas = ChartCanvas::default();
    canvas.watermark.mode = crate::model::canvas::WatermarkMode::Replay;
    let mut replaying = Harness::new(canvas);
    replaying.replay_active = true;
    let (live, rect) = settled(&mut replaying);
    let snapshot = dumped(&snapshot_shapes(&replaying, rect, false));
    assert!(snapshot.iter().any(|shape| shape.contains("Replay")));
    let (only_live, only_snapshot) = differences(&live, &snapshot_shapes(&replaying, rect, false));
    assert!(only_snapshot.is_empty());
    // (The plot's own replay badge only shows on a narrow chart; this one is wide.)
    assert!(only_live.is_empty(), "{only_live:?}");
}

#[test]
fn the_locked_cursor_line_is_not_part_of_the_image() {
    let mut harness = Harness::new(ChartCanvas::default());
    harness.card.set_time_lock(Some(250 * HOUR));
    let (live, rect) = settled(&mut harness);
    let (only_live, only_snapshot) = differences(&live, &snapshot_shapes(&harness, rect, false));
    assert!(only_snapshot.is_empty());
    assert!(!only_live.is_empty());
}

// MARK: The image

fn scene_of(harness: &Harness, scale: SnapshotScale) -> crate::service::snapshot_raster::Scene {
    let size = harness.card.snapshot_size();
    let dimensions = SnapshotDimensions::fit(size.x, size.y, scale).expect("a size");
    harness
        .card
        .snapshot_scene(&input(harness, false), dimensions, Color32::BLACK)
        .expect("a chart to draw")
}

#[test]
fn a_snapshot_is_an_image_of_the_chart_at_the_scale_asked_for() {
    let mut harness = Harness::new(ChartCanvas::default());
    settled(&mut harness);
    let size = harness.card.snapshot_size();
    assert_eq!((size.x, size.y), (760.0, 520.0), "the chart's own size");
    let one = crate::service::snapshot_raster::rasterize(&scene_of(&harness, SnapshotScale::X1))
        .expect("drawn");
    let two = crate::service::snapshot_raster::rasterize(&scene_of(&harness, SnapshotScale::X2))
        .expect("drawn");
    assert_eq!(one.size, [760, 520]);
    assert_eq!(two.size, [1520, 1040]);
    // The chart is not blank, and it holds both colours of a candle.
    let count = |image: &ColorImage, color: Color32| {
        image
            .pixels
            .iter()
            .filter(|pixel| {
                pixel.r().abs_diff(color.r()) < 4
                    && pixel.g().abs_diff(color.g()) < 4
                    && pixel.b().abs_diff(color.b()) < 4
            })
            .count()
    };
    let [ur, ug, ub] = harness.card.appearance().bullish;
    let up_color = Color32::from_rgb(ur, ug, ub);
    let up = count(&two, up_color);
    assert!(up > 5_000, "the sample market only rises: {up}");
    // Drawn again at twice the scale, a candle covers four times the pixels, not the same ones.
    let ratio = count(&two, up_color) as f32 / count(&one, up_color) as f32;
    assert!((3.4..4.6).contains(&ratio), "{ratio}");
}

#[test]
fn the_backdrop_is_the_canvas() {
    let mut canvas = ChartCanvas::default();
    canvas.background.mode = crate::model::canvas::BackgroundMode::Gradient;
    canvas.background.top = Some([255, 0, 0, 255]);
    canvas.background.bottom = Some([0, 0, 255, 255]);
    let mut harness = Harness::new(canvas);
    settled(&mut harness);
    let image = crate::service::snapshot_raster::rasterize(&scene_of(&harness, SnapshotScale::X1))
        .expect("drawn");
    let width = image.size[0];
    let top = image.pixels[2 * width + 2];
    let bottom = image.pixels[(image.size[1] - 3) * width + 2];
    assert!(top.r() > 240 && top.b() < 15, "{top:?}");
    assert!(bottom.b() > 240 && bottom.r() < 15, "{bottom:?}");
}

// MARK: Nothing changes

#[test]
fn taking_a_snapshot_changes_nothing_in_the_chart() {
    let mut harness = Harness::new(ChartCanvas::default());
    harness.card.appearance_mut().style.chart_type = ChartType::Renko;
    harness.card.set_visible_candles(80);
    settled(&mut harness);
    let cached = harness.card.constructed.clone().expect("renko was built");
    let viewport = harness.card.saved_viewport();
    let panes = harness.card.pane_ids.clone();
    let near_oldest = harness.card.near_oldest;
    let extent = format!("{:?}", harness.card.script_extent);
    let candles = harness.candles.clone();
    for scale in SnapshotScale::ALL {
        drop(scene_of(&harness, scale));
    }
    assert!(Arc::ptr_eq(
        &cached,
        harness.card.constructed.as_ref().expect("still built")
    ));
    assert_eq!(harness.card.saved_viewport(), viewport);
    assert_eq!(harness.card.pane_ids, panes);
    assert_eq!(format!("{:?}", harness.card.script_extent), extent);
    assert_eq!(harness.card.near_oldest, near_oldest);
    assert_eq!(harness.candles, candles);
}

#[test]
fn a_chart_with_nothing_to_draw_has_no_snapshot() {
    let mut harness = Harness::new(ChartCanvas::default());
    settled(&mut harness);
    harness.candles.clear();
    let dimensions = SnapshotDimensions::fit(760.0, 520.0, SnapshotScale::X1).expect("a size");
    assert_eq!(
        harness
            .card
            .snapshot_scene(&input(&harness, false), dimensions, Color32::BLACK)
            .err(),
        Some(SnapshotError::Empty)
    );
}

/// Prints how long each stage of a snapshot takes and how big the image is. Run with
/// `cargo test --release snapshot_cost -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement, not a check"]
fn snapshot_cost() {
    use std::time::Instant;
    let mut harness = with_panes(ChartCanvas::default());
    harness.indicators = everything_on();
    harness.drawings = vec![trend_line()];
    with_account(&mut harness);
    harness.candles = super::canvas_tests::series(500);
    settled(&mut harness);
    for scale in SnapshotScale::ALL {
        let size = harness.card.snapshot_size();
        let dimensions = SnapshotDimensions::fit(size.x, size.y, scale).expect("a size");
        let started = Instant::now();
        let scene = harness
            .card
            .snapshot_scene(&input(&harness, true), dimensions, Color32::BLACK)
            .expect("a scene");
        let frozen = started.elapsed();
        let started = Instant::now();
        let image = crate::service::snapshot_raster::rasterize(&scene).expect("drawn");
        let rasterized = started.elapsed();
        let started = Instant::now();
        let png = crate::service::snapshot_export::encode_png(&image).expect("encoded");
        let encoded = started.elapsed();
        eprintln!(
            "{}×{}: freeze+tessellate {frozen:?} (UI thread), rasterize {rasterized:?}, png {encoded:?}, {} KiB png, {} MiB raw",
            dimensions.width,
            dimensions.height,
            png.len() / 1024,
            image.pixels.len() * 4 / (1024 * 1024),
        );
    }
}

// MARK: A look at the new tools

/// The lines, channels and pitchforks on a chart, written as PNGs for a person to look at.
/// Run by hand: `DEGENVIEW_GALLERY_DIR=<dir> cargo test --lib gallery -- --ignored`.
#[test]
#[ignore = "writes images for a person to look at"]
fn gallery_of_the_lines_channels_and_pitchforks() {
    use crate::model::{Anchor, Placement};
    let Some(dir) = std::env::var_os("DEGENVIEW_GALLERY_DIR") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    let at = |bar: i64, price: f64| Anchor {
        time_ms: bar * HOUR,
        price,
    };
    // Anchors in the last stretch of `series(300)`: bars 215..290, prices around 200..270.
    let anchors = [
        at(236, 230.0),
        at(262, 262.0),
        at(250, 218.0),
        at(285, 240.0),
    ];
    let make = |tool: Tool, id: u64, text: &str| {
        let count = match tool.spec().placement {
            Placement::Points(count) => usize::from(count),
            _ => 1,
        };
        let mut drawing =
            Drawing::build(tool, id, &anchors[..count], tool.default_style()).expect("builds");
        drawing.set_text(text);
        drawing
    };
    let sets: [(&str, Vec<(Tool, &str)>); 5] = [
        (
            "lines-1",
            vec![
                (Tool::TrendLine, "support"),
                (Tool::Ray, ""),
                (Tool::InfoLine, ""),
            ],
        ),
        (
            "lines-2",
            vec![
                (Tool::ExtendedLine, "extended"),
                (Tool::TrendAngle, ""),
                (Tool::HorizontalLine, "level"),
                (Tool::VerticalLine, "event"),
            ],
        ),
        (
            "lines-3",
            vec![(Tool::HorizontalRay, "ray"), (Tool::Crossline, "")],
        ),
        (
            "channels",
            vec![
                (Tool::ParallelChannel, "channel"),
                (Tool::RegressionTrend, ""),
            ],
        ),
        (
            "channels-2",
            vec![(Tool::FlatChannel, "flat"), (Tool::DisjointChannel, "")],
        ),
    ];
    let forks: Vec<(String, Vec<(Tool, &str)>)> = [
        Tool::Pitchfork,
        Tool::SchiffPitchfork,
        Tool::ModifiedSchiffPitchfork,
        Tool::InsidePitchfork,
    ]
    .into_iter()
    .map(|tool| (format!("fork-{tool:?}"), vec![(tool, "")]))
    .collect();
    let all: Vec<(String, Vec<(Tool, &str)>)> = sets
        .into_iter()
        .map(|(name, tools)| (name.to_owned(), tools))
        .chain(forks)
        .collect();
    use crate::ui::theme::{self, AppTheme};
    // Dark first, then a few again in the light theme (the faint invitation and the white-on-colour
    // tags are what a light chart puts at risk).
    for light in [false, true] {
        theme::apply(
            &egui::Context::default(),
            if light {
                AppTheme::Light
            } else {
                AppTheme::Dark
            },
        );
        for (name, tools) in &all {
            if light && !["lines-2", "channels", "fork-Pitchfork"].contains(&name.as_str()) {
                continue;
            }
            let mut harness = Harness::new(ChartCanvas::default());
            harness.drawings = tools
                .iter()
                .enumerate()
                .map(|(index, (tool, text))| make(*tool, index as u64 + 1, text))
                .collect();
            settled(&mut harness);
            let size = harness.card.snapshot_size();
            let dimensions =
                SnapshotDimensions::fit(size.x, size.y, SnapshotScale::X2).expect("a size");
            let scene = harness
                .card
                .snapshot_scene(&input(&harness, false), dimensions, Color32::BLACK)
                .expect("a scene");
            let image = crate::service::snapshot_raster::rasterize(&scene).expect("drawn");
            let png = crate::service::snapshot_export::encode_png(&image).expect("encoded");
            let suffix = if light { "-light" } else { "" };
            std::fs::write(dir.join(format!("{name}{suffix}.png")), png).expect("written");
        }
    }
    theme::apply(&egui::Context::default(), AppTheme::Dark);
}

/// The text and note tools on a chart, written as PNGs for a person to look at.
/// Run by hand: `DEGENVIEW_GALLERY_DIR=<dir> cargo test --lib gallery -- --ignored`.
#[test]
#[ignore = "writes images for a person to look at"]
fn gallery_of_the_text_and_note_tools() {
    use crate::model::{Anchor, DrawingStyle};
    use crate::ui::theme::{self, AppTheme};
    let Some(dir) = std::env::var_os("DEGENVIEW_GALLERY_DIR") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    let at = |bar: i64, price: f64| Anchor {
        time_ms: bar * HOUR,
        price,
    };
    let make = |tool: Tool, id: u64, anchor: Anchor, text: &str, tweak: fn(&mut DrawingStyle)| {
        let mut style = tool.default_style();
        tweak(&mut style);
        let mut drawing = Drawing::build(tool, id, &[anchor], style).expect("builds");
        drawing.set_text(text);
        drawing
    };
    let plain: fn(&mut DrawingStyle) = |_| {};
    let sets: Vec<(&str, Vec<Drawing>, Vec<u64>)> = vec![
        (
            "notes-1",
            vec![
                make(Tool::Text, 1, at(222, 258.0), "Breakout above 250", plain),
                make(Tool::Comment, 2, at(262, 244.0), "Retest", plain),
                make(Tool::PriceLabel, 3, at(252, 206.0), "", plain),
                make(Tool::FlagMark, 4, at(276, 236.0), "", plain),
                make(
                    Tool::Pin,
                    5,
                    at(236, 214.0),
                    "Earnings gap: watch the close above the prior high",
                    plain,
                ),
            ],
            vec![5],
        ),
        (
            "notes-2",
            vec![
                make(
                    Tool::Text,
                    1,
                    at(224, 262.0),
                    "A boxed note that wraps its long text inside a fixed width",
                    |style| {
                        style.fill.enabled = true;
                        style.border.enabled = true;
                        style.wrap = true;
                        style.box_width = 150.0;
                        style.text.bold = true;
                    },
                ),
                make(Tool::Comment, 2, at(262, 244.0), "Two\nlines", plain),
                make(Tool::Pin, 3, at(240, 218.0), "closed pin", plain),
                make(Tool::Pin, 4, at(268, 222.0), "open pin", plain),
            ],
            vec![4],
        ),
    ];
    for light in [false, true] {
        theme::apply(
            &egui::Context::default(),
            if light {
                AppTheme::Light
            } else {
                AppTheme::Dark
            },
        );
        for (name, drawings, open) in &sets {
            let mut harness = Harness::new(ChartCanvas::default());
            harness.drawings = drawings.clone();
            settled(&mut harness);
            let size = harness.card.snapshot_size();
            let dimensions =
                SnapshotDimensions::fit(size.x, size.y, SnapshotScale::X2).expect("a size");
            let mut snapshot = input(&harness, false);
            snapshot.open_notes = open;
            let scene = harness
                .card
                .snapshot_scene(&snapshot, dimensions, Color32::BLACK)
                .expect("a scene");
            let image = crate::service::snapshot_raster::rasterize(&scene).expect("drawn");
            let png = crate::service::snapshot_export::encode_png(&image).expect("encoded");
            let suffix = if light { "-light" } else { "" };
            std::fs::write(dir.join(format!("{name}{suffix}.png")), png).expect("written");
        }
    }
    theme::apply(&egui::Context::default(), AppTheme::Dark);
}

/// The notes with a leader or a tail, and the signposts, on a chart.
#[test]
#[ignore = "writes images for a person to look at"]
fn gallery_of_the_two_point_notes() {
    use crate::model::drawing_style::Emoji;
    use crate::model::{Anchor, DrawingStyle};
    use crate::ui::theme::{self, AppTheme};
    let Some(dir) = std::env::var_os("DEGENVIEW_GALLERY_DIR") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    let at = |bar: i64, price: f64| Anchor {
        time_ms: bar * HOUR,
        price,
    };
    let make =
        |tool: Tool, id: u64, anchors: [Anchor; 2], text: &str, tweak: fn(&mut DrawingStyle)| {
            let mut style = tool.default_style();
            tweak(&mut style);
            let mut drawing = Drawing::build(tool, id, &anchors, style).expect("builds");
            drawing.set_text(text);
            drawing
        };
    let plain: fn(&mut DrawingStyle) = |_| {};
    let sets: Vec<(&str, Vec<Drawing>)> = vec![
        (
            "notes-3",
            vec![
                make(
                    Tool::Note,
                    1,
                    [at(236, 226.0), at(250, 258.0)],
                    "Note on a level",
                    plain,
                ),
                make(
                    Tool::PriceNote,
                    2,
                    [at(262, 244.0), at(272, 210.0)],
                    "",
                    plain,
                ),
                make(
                    Tool::PriceNote,
                    3,
                    [at(226, 204.0), at(240, 192.0)],
                    "support",
                    plain,
                ),
                make(
                    Tool::Callout,
                    4,
                    [at(280, 238.0), at(268, 262.0)],
                    "Callout",
                    plain,
                ),
            ],
        ),
        (
            "notes-4",
            vec![
                make(
                    Tool::Signpost,
                    1,
                    [at(240, 222.0), at(240, 252.0)],
                    "Earnings",
                    plain,
                ),
                make(
                    Tool::Signpost,
                    2,
                    [at(262, 250.0), at(262, 214.0)],
                    "FOMC",
                    |style| {
                        style.emoji_shown = true;
                        style.emoji = Emoji::Crown;
                    },
                ),
                make(
                    Tool::Signpost,
                    3,
                    [at(280, 240.0), at(280, 266.0)],
                    "Halving",
                    |style| {
                        style.emoji_shown = true;
                        style.emoji = Emoji::Skull;
                        style.emoji_fill = [242, 54, 69, 255];
                        style.fill.color = [242, 54, 69, 255];
                    },
                ),
            ],
        ),
    ];
    for light in [false, true] {
        theme::apply(
            &egui::Context::default(),
            if light {
                AppTheme::Light
            } else {
                AppTheme::Dark
            },
        );
        for (name, drawings) in &sets {
            let mut harness = Harness::new(ChartCanvas::default());
            harness.drawings = drawings.clone();
            settled(&mut harness);
            let size = harness.card.snapshot_size();
            let dimensions =
                SnapshotDimensions::fit(size.x, size.y, SnapshotScale::X2).expect("a size");
            let scene = harness
                .card
                .snapshot_scene(&input(&harness, false), dimensions, Color32::BLACK)
                .expect("a scene");
            let image = crate::service::snapshot_raster::rasterize(&scene).expect("drawn");
            let png = crate::service::snapshot_export::encode_png(&image).expect("encoded");
            let suffix = if light { "-light" } else { "" };
            std::fs::write(dir.join(format!("{name}{suffix}.png")), png).expect("written");
        }
    }
    theme::apply(&egui::Context::default(), AppTheme::Dark);
}

/// Tables on a chart, written as PNGs for a person to look at.
#[test]
#[ignore = "writes images for a person to look at"]
fn gallery_of_the_table() {
    use crate::model::table::ScreenAnchor;
    use crate::model::{Anchor, DrawingStyle, Geometry};
    use crate::ui::theme::{self, AppTheme};
    let Some(dir) = std::env::var_os("DEGENVIEW_GALLERY_DIR") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    let at = |bar: i64, price: f64| Anchor {
        time_ms: bar * HOUR,
        price,
    };
    let table = |id: u64, anchor: Anchor, cells: &[&str], tweak: fn(&mut DrawingStyle)| {
        let mut style = Tool::Table.default_style();
        tweak(&mut style);
        let mut drawing = Drawing::build(Tool::Table, id, &[anchor], style).expect("builds");
        for (index, text) in cells.iter().enumerate() {
            drawing.set_table_cell(index / 3, index % 3, text);
        }
        drawing
    };
    let plain: fn(&mut DrawingStyle) = |_| {};
    let mut pinned = table(
        2,
        at(250, 200.0),
        &["Pinned", "to the", "screen"],
        |style| {
            style.text.bold = true;
            style.text.align = crate::model::drawing_style::HorizontalAlign::Center;
        },
    );
    if let Some(Geometry::Table(data)) = pinned.geometry_mut() {
        data.screen = Some(ScreenAnchor { x: 0.62, y: 0.08 });
    }
    let sets: Vec<(&str, Vec<Drawing>)> = vec![
        (
            "table-1",
            vec![
                table(
                    1,
                    at(224, 262.0),
                    &[
                        "Level",
                        "Price",
                        "Note",
                        "Entry",
                        "238.50",
                        "Break of range",
                        "Stop",
                        "226.00",
                        "A longer note that has to wrap inside its narrow column and make the row taller",
                    ],
                    plain,
                ),
                pinned,
            ],
        ),
        (
            "table-2",
            vec![table(
                1,
                at(236, 250.0),
                &["", "", "", "", "no border", "", "", "", ""],
                |style| {
                    style.border.enabled = false;
                    style.fill.enabled = false;
                },
            )],
        ),
    ];
    for light in [false, true] {
        theme::apply(
            &egui::Context::default(),
            if light {
                AppTheme::Light
            } else {
                AppTheme::Dark
            },
        );
        for (name, drawings) in &sets {
            let mut harness = Harness::new(ChartCanvas::default());
            harness.drawings = drawings.clone();
            settled(&mut harness);
            let size = harness.card.snapshot_size();
            let dimensions =
                SnapshotDimensions::fit(size.x, size.y, SnapshotScale::X2).expect("a size");
            let scene = harness
                .card
                .snapshot_scene(&input(&harness, false), dimensions, Color32::BLACK)
                .expect("a scene");
            let image = crate::service::snapshot_raster::rasterize(&scene).expect("drawn");
            let png = crate::service::snapshot_export::encode_png(&image).expect("encoded");
            let suffix = if light { "-light" } else { "" };
            std::fs::write(dir.join(format!("{name}{suffix}.png")), png).expect("written");
        }
    }
    theme::apply(&egui::Context::default(), AppTheme::Dark);
}

/// Pictures on a chart, written as PNGs for a person to look at: one loaded, one faded, one that
/// is not in the store (a placeholder).
#[test]
#[ignore = "writes images for a person to look at"]
fn gallery_of_the_image() {
    use crate::model::image_content::{Content, ImageContent};
    use crate::model::{Anchor, Geometry};
    use crate::ui::theme::{self, AppTheme};
    use std::collections::HashMap;
    use std::sync::Arc;
    let Some(dir) = std::env::var_os("DEGENVIEW_GALLERY_DIR") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    let at = |bar: i64, price: f64| Anchor {
        time_ms: bar * HOUR,
        price,
    };
    let picture = |asset: &str, name: &str| {
        Content::Image(ImageContent::new(asset.repeat(64), name, [160, 100]))
    };
    let image = |id: u64, at: Anchor, content: &Content| {
        Drawing::build_with(Tool::Image, id, &[at], Tool::Image.default_style(), content)
            .expect("builds")
    };
    let loaded = image(1, at(205, 275.0), &picture("a", "gradient.png"));
    let mut faded = image(2, at(232, 262.0), &picture("a", "gradient.png"));
    if let Some(Geometry::Image(data)) = faded.geometry_mut() {
        data.opacity = 0.35;
    }
    let missing = image(3, at(215, 235.0), &picture("b", "still-syncing.png"));
    let gradient = egui::ColorImage::from_rgba_unmultiplied(
        [160, 100],
        &(0..160 * 100)
            .flat_map(|index| {
                let (x, y) = (index % 160, index / 160);
                [(x * 255 / 159) as u8, (y * 255 / 99) as u8, 180, 255]
            })
            .collect::<Vec<u8>>(),
    );
    let images: HashMap<String, Arc<egui::ColorImage>> =
        HashMap::from([("a".repeat(64), Arc::new(gradient))]);
    for light in [false, true] {
        theme::apply(
            &egui::Context::default(),
            if light {
                AppTheme::Light
            } else {
                AppTheme::Dark
            },
        );
        let mut harness = Harness::new(ChartCanvas::default());
        harness.drawings = vec![loaded.clone(), faded.clone(), missing.clone()];
        settled(&mut harness);
        let size = harness.card.snapshot_size();
        let dimensions =
            SnapshotDimensions::fit(size.x, size.y, SnapshotScale::X2).expect("a size");
        let mut snapshot = input(&harness, false);
        snapshot.images = Some(&images);
        let scene = harness
            .card
            .snapshot_scene(&snapshot, dimensions, Color32::BLACK)
            .expect("a scene");
        let image = crate::service::snapshot_raster::rasterize(&scene).expect("drawn");
        let png = crate::service::snapshot_export::encode_png(&image).expect("encoded");
        let suffix = if light { "-light" } else { "" };
        std::fs::write(dir.join(format!("image-1{suffix}.png")), png).expect("written");
    }
    theme::apply(&egui::Context::default(), AppTheme::Dark);
}

/// Post cards on a chart, written as PNGs for a person to look at: a bare link, a card with a
/// fetched preview, one with a long preview, and a small one that has to cut its words.
#[test]
#[ignore = "writes images for a person to look at"]
fn gallery_of_the_cards() {
    use crate::model::Anchor;
    use crate::model::image_content::Content;
    use crate::model::link_card::{Link, LinkCard, LinkMeta};
    use crate::ui::theme::{self, AppTheme};
    let Some(dir) = std::env::var_os("DEGENVIEW_GALLERY_DIR") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    let at = |bar: i64, price: f64| Anchor {
        time_ms: bar * HOUR,
        price,
    };
    let card = |tool: Tool, id: u64, anchor: Anchor, link: &str, meta: Option<LinkMeta>, size| {
        let mut card = LinkCard::new(Link::parse(link).expect("a link"));
        card.meta = meta;
        if let Some(size) = size {
            card.size = size;
        }
        Drawing::build_with(
            tool,
            id,
            &[anchor],
            tool.default_style(),
            &Content::Link(card),
        )
        .expect("builds")
    };
    let picture = "c".repeat(64);
    let post_meta = LinkMeta {
        title: "Developers".into(),
        avatar: Some(picture.clone()),
        author: "@XDevelopers".into(),
        excerpt: "Fish & chips for everyone: the range held three times, and the third retest was the one to buy. Stops under the low.".into(),
        fetched_at_ms: 1,
    };
    let long_meta = LinkMeta {
        title: "BTC range break: what to watch next".into(),
        author: "@degenview".into(),
        excerpt: "Price reclaimed the range high on rising volume. A retest of that level would confirm the move.".into(),
        fetched_at_ms: 1,
        avatar: Some(picture.clone()),
    };
    let bare = card(
        Tool::Post,
        1,
        at(186, 276.0),
        "https://x.com/degenview/status/1750000000000000000",
        None,
        Some([260.0, 110.0]),
    );
    let fetched = card(
        Tool::Post,
        2,
        at(240, 276.0),
        "https://x.com/XDevelopers/status/1750000000000000001",
        Some(post_meta),
        Some([320.0, 150.0]),
    );
    let long = card(
        Tool::Post,
        3,
        at(196, 226.0),
        "https://x.com/degenview/status/1750000000000000002",
        Some(long_meta),
        Some([300.0, 130.0]),
    );
    let small = card(
        Tool::Post,
        4,
        at(262, 226.0),
        "https://x.com/i/status/1750000000000000003",
        None,
        Some([180.0, 96.0]),
    );
    for light in [false, true] {
        theme::apply(
            &egui::Context::default(),
            if light {
                AppTheme::Light
            } else {
                AppTheme::Dark
            },
        );
        let mut harness = Harness::new(ChartCanvas::default());
        harness.drawings = vec![bare.clone(), fetched.clone(), long.clone(), small.clone()];
        settled(&mut harness);
        let size = harness.card.snapshot_size();
        let dimensions =
            SnapshotDimensions::fit(size.x, size.y, SnapshotScale::X2).expect("a size");
        let mut snapshot = input(&harness, false);
        let pictures = std::collections::HashMap::from([(
            picture.clone(),
            std::sync::Arc::new(egui::ColorImage::from_rgba_unmultiplied(
                [48, 48],
                &(0..48 * 48)
                    .flat_map(|index| {
                        let (x, y) = (index % 48, index / 48);
                        [(x * 5) as u8, 90, (y * 5) as u8, 255]
                    })
                    .collect::<Vec<u8>>(),
            )),
        )]);
        snapshot.images = Some(&pictures);
        let scene = harness
            .card
            .snapshot_scene(&snapshot, dimensions, Color32::BLACK)
            .expect("a scene");
        let image = crate::service::snapshot_raster::rasterize(&scene).expect("drawn");
        let png = crate::service::snapshot_export::encode_png(&image).expect("encoded");
        let suffix = if light { "-light" } else { "" };
        std::fs::write(dir.join(format!("cards-1{suffix}.png")), png).expect("written");
    }
    theme::apply(&egui::Context::default(), AppTheme::Dark);
}
