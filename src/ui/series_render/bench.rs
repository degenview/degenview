//! Frame timings of the three order-flow painters on dense data, for a person to run in release:
//! `cargo test --release --lib series_render::bench -- --ignored --nocapture`.
//!
//! Each frame is egui's own work for the shapes (layout of every number and letter included) and the
//! tessellation of them, which is what a screen redraw costs apart from the GPU. Nothing is asserted
//! but a loose ceiling.

use super::SeriesFrame;
use crate::model::chart_look::Palette;
use crate::model::chart_type::{ChartStyle, ChartType};
use crate::model::market_profile::feed::ProfileFeed;
use crate::model::market_profile::flow::{CandleBook, FlowKline};
use crate::model::market_profile::trades::{Trade, TradeBook};
use crate::model::market_profile::{footprint, svp, tpo};
use crate::model::{Kline, PriceFormat};
use crate::ui::chart_card::PriceScale;
use eframe::egui::{self, Pos2, Rect};
use std::sync::Arc;
use std::time::Instant;

const MIN: i64 = 60_000;
const HOUR: i64 = 60 * MIN;

fn price(i: i64) -> f64 {
    let t = i as f64;
    60_000.0 + 1_500.0 * (t / 900.0).sin() + 120.0 * (t * 0.37).sin()
}

fn flow(count: i64, size: i64) -> Vec<FlowKline> {
    (0..count)
        .map(|i| {
            let (open, close) = (price(i * size / MIN), price((i + 1) * size / MIN));
            FlowKline {
                kline: Kline {
                    open_time: i * size,
                    close_time: (i + 1) * size - 1,
                    open,
                    high: open.max(close) + 20.0,
                    low: open.min(close) - 20.0,
                    close,
                    volume: 5.0 + (i % 11) as f64,
                },
                taker_buy: Some(2.0 + (i % 5) as f64),
            }
        })
        .collect()
}

fn frame_ms(
    chart_type: ChartType,
    bars: &[Kline],
    feed: &ProfileFeed,
    style: &ChartStyle,
    (width, height): (f32, f32),
    low_high: (f64, f64),
) -> f64 {
    let ctx = egui::Context::default();
    crate::ui::theme::install_offscreen(&ctx);
    let plot = Rect::from_min_max(Pos2::ZERO, Pos2::new(width, height));
    let scale = PriceScale::linear(low_high.0, low_high.1, 0.0, height);
    let slot = width / bars.len() as f32;
    let one = |ctx: &egui::Context| {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(plot),
                ..Default::default()
            },
            |ui| {
                let x_for = |index: usize| slot * (index as f32 + 0.5);
                super::paint(&SeriesFrame {
                    painter: ui.painter(),
                    plot,
                    bars,
                    synthetic: None,
                    box_size: None,
                    previous_close: None,
                    slot_width: slot,
                    x_for: &x_for,
                    scale: &scale,
                    palette: Palette::new([38, 166, 154], [239, 83, 80]),
                    style,
                    chart_type,
                    recolor: None,
                    price_format: PriceFormat::AUTO,
                    profiles: Some(feed),
                });
            },
        );
        let meshes = ctx.tessellate(std::mem::take(&mut output.shapes), 2.0);
        output.textures_delta.clear();
        meshes.len()
    };
    // Warm the font atlas, then time a run.
    for _ in 0..3 {
        one(&ctx);
    }
    let frames = 20;
    let start = Instant::now();
    for _ in 0..frames {
        one(&ctx);
    }
    start.elapsed().as_secs_f64() * 1_000.0 / f64::from(frames)
}

#[test]
#[ignore = "timing run"]
fn bench_frame_times_of_the_order_flow_painters() {
    let mut style = ChartStyle::default();
    // Forty hourly bars in view, a tall plot.
    let chart: Vec<Kline> = flow(40 * 60, MIN)
        .chunks(60)
        .map(|hour| Kline {
            open_time: hour[0].kline.open_time,
            close_time: hour[59].kline.close_time,
            open: hour[0].kline.open,
            high: hour.iter().map(|c| c.kline.high).fold(f64::MIN, f64::max),
            low: hour.iter().map(|c| c.kline.low).fold(f64::MAX, f64::min),
            close: hour[59].kline.close,
            volume: hour.iter().map(|c| c.kline.volume).sum(),
        })
        .collect();
    let (low, high) = chart.iter().fold((f64::MAX, f64::MIN), |(l, h), b| {
        (l.min(b.low), h.max(b.high))
    });
    let range = (low - 50.0, high + 50.0);
    let mut minutes = CandleBook::new(MIN, 100_000);
    minutes.insert(flow(40 * 60, MIN));
    let mut fives = CandleBook::new(5 * MIN, 100_000);
    fives.insert(flow(40 * 12, 5 * MIN));
    let mut trades = TradeBook::with_capacity(1_000_000);
    trades.insert(
        &(0..200_000u64)
            .map(|i| Trade {
                id: i + 1,
                time_ms: 1 + (i as i64) * (40 * HOUR) / 200_000,
                price: price((i as i64) / 80) + (i % 30) as f64 * 0.5,
                quantity: 0.01,
                buyer_is_maker: i % 3 == 0,
            })
            .collect::<Vec<_>>(),
    );
    // Dense footprint: rows of a few ticks, every cell wide enough for numbers.
    style.footprint.row_mode = crate::model::chart_type::RowMode::Manual;
    style.footprint.ticks_per_row = 400;
    let footprint_set = footprint::build_set(
        footprint::SetInput {
            bars: &chart,
            bar_ms: HOUR,
            candles: Some(&minutes),
            trades: Some(&mut trades),
            exchange_tick: Some(0.01),
            shape: style.footprint.shape(),
        },
        &mut footprint::Cache::default(),
    );
    let rows: usize = footprint_set
        .bars
        .iter()
        .flatten()
        .map(|bar| bar.cells.len())
        .sum();
    let feed = ProfileFeed {
        footprint: Some(Arc::new(footprint_set)),
        ..ProfileFeed::default()
    };
    let ms = frame_ms(
        ChartType::VolumeFootprint,
        &chart,
        &feed,
        &style,
        (2400.0, 1400.0),
        range,
    );
    println!("footprint, 40 bars, {rows} cells, 2400x1400: {ms:.1} ms a frame");
    let mut sparse = style;
    sparse.footprint.ticks_per_row = 4000;
    let sparse_feed = ProfileFeed {
        footprint: Some(Arc::new(footprint::build_set(
            footprint::SetInput {
                bars: &chart,
                bar_ms: HOUR,
                candles: Some(&minutes),
                trades: None,
                exchange_tick: Some(0.01),
                shape: sparse.footprint.shape(),
            },
            &mut footprint::Cache::default(),
        ))),
        ..ProfileFeed::default()
    };
    let sparse_ms = frame_ms(
        ChartType::VolumeFootprint,
        &chart,
        &sparse_feed,
        &sparse,
        (2400.0, 1400.0),
        range,
    );
    println!("footprint, coarse rows: {sparse_ms:.1} ms a frame");

    style.tpo.row_mode = crate::model::chart_type::RowMode::Manual;
    style.tpo.ticks_per_row = 400;
    style.tpo.block_minutes = 5;
    let tpo_set = tpo::build_set(
        tpo::SetInput {
            bars: &chart,
            bar_ms: HOUR,
            candles: &fives,
            exchange_tick: Some(0.01),
            shape: style.tpo.shape(),
            edits: &[],
            until_ms: i64::MAX,
            now_ms: i64::MAX,
        },
        &mut tpo::Cache::default(),
    );
    let letters: usize = tpo_set
        .profiles
        .iter()
        .flat_map(|p| p.rows.iter())
        .map(|row| row.blocks.len())
        .sum();
    let tpo_feed = ProfileFeed {
        tpo: Some(Arc::new(tpo_set)),
        ..ProfileFeed::default()
    };
    let tpo_ms = frame_ms(
        ChartType::TimePriceOpportunity,
        &chart,
        &tpo_feed,
        &style,
        (2400.0, 1400.0),
        range,
    );
    println!("TPO, 5-minute blocks, {letters} letters, 2400x1400: {tpo_ms:.1} ms a frame");

    style.svp.developing_poc = true;
    style.svp.developing_value_area = true;
    let svp_set = svp::build_set(
        svp::SetInput {
            bars: &chart,
            bar_ms: HOUR,
            candles: &minutes,
            exchange_tick: Some(0.01),
            shape: style.svp.shape(),
            until_ms: i64::MAX,
            now_ms: i64::MAX,
        },
        &mut svp::Cache::default(),
    );
    let svp_feed = ProfileFeed {
        svp: Some(Arc::new(svp_set)),
        ..ProfileFeed::default()
    };
    let svp_ms = frame_ms(
        ChartType::SessionVolumeProfile,
        &chart,
        &svp_feed,
        &style,
        (2400.0, 1400.0),
        range,
    );
    println!("session profile, developing levels on, 2400x1400: {svp_ms:.1} ms a frame");
    assert!(
        ms < 500.0 && tpo_ms < 500.0 && svp_ms < 500.0,
        "a loose ceiling"
    );
}
