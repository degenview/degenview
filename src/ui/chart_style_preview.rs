//! The preview under the chart-type picker: the selected type drawn with the chart's own painter
//! code (`series_render::paint`) over sample prices, in the colours and styles the user has set.
//!
//! Sample prices rather than the chart's: the preview must read the same on any market, so a
//! Renko box size of 500 set for Bitcoin still gives a readable figure on prices near 100. Only
//! the *size* of a price-based construction is replaced; its source, wicks, line count, reversal
//! and look are the user's.

use super::chart_card::PriceScale;
use super::kit;
use super::series_render::{self, SeriesFrame};
use super::theme;
use crate::model::chart_look::Palette;
use crate::model::chart_type::{ChartType, SizeMethod, SizeRule};
use crate::model::market_profile::feed::ProfileFeed;
use crate::model::market_profile::flow::{CandleBook, FlowKline};
use crate::model::market_profile::footprint::{self, Cache, SetInput};
use crate::model::market_profile::svp;
use crate::model::market_profile::tpo;
use crate::model::market_profile::trades::{Trade, TradeBook};
use crate::model::synthetic::{Construction, SyntheticSeries, build_with};
use crate::model::{ChartAppearance, Kline};
use eframe::egui::{Rect, Sense, Ui, vec2};

const HEIGHT: f32 = 104.0;

/// Roughly 26 hourly candles: a rise, a pull-back, a sharp reversal and a recovery, one doji, and
/// volumes that differ so Volume Candles shows its widths.
pub fn sample_candles() -> Vec<Kline> {
    let mut close = 100.0_f64;
    (0..26)
        .map(|index| {
            let open = close;
            let wave = (index as f64 * 0.55).sin() * 3.4 + 0.45;
            close = if index == 11 { open } else { open + wave };
            let spread = 0.9 + (index % 3) as f64 * 0.7;
            Kline {
                open_time: index as i64 * 3_600_000,
                close_time: (index as i64 + 1) * 3_600_000 - 1,
                open,
                high: open.max(close) + spread,
                low: open.min(close) - spread,
                close,
                volume: 20.0 + ((index * 37) % 11) as f64 * 9.0,
            }
        })
        .collect()
}

/// The bars the footprint preview shows: the last few sample candles, so each has room for cells.
const FOOTPRINT_BARS: usize = 6;

/// The tick the sample prices trade on.
const SAMPLE_TICK: f64 = 0.5;

/// A deterministic footprint over the sample candles: a trade at every tick of each bar's range, the
/// sizes from a fixed sequence, so it reads the same every time and on every market. Built by the
/// same builder as a real chart's, only from made-up trades (the preview says nothing of any market).
pub fn sample_footprint(appearance: &ChartAppearance) -> ProfileFeed {
    let bars: Vec<Kline> = {
        let all = sample_candles();
        all[all.len() - FOOTPRINT_BARS..].to_vec()
    };
    let mut book = TradeBook::default();
    let mut id = 1u64;
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let mut next = |bound: u64| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed % bound
    };
    let mut trades = Vec::new();
    // One trade before the first bar, so the book is complete from the first bar's open.
    trades.push(Trade {
        id,
        time_ms: bars[0].open_time - 1,
        price: bars[0].open,
        quantity: 0.01,
        buyer_is_maker: false,
    });
    id += 1;
    for bar in &bars {
        let mut price = (bar.low / SAMPLE_TICK).floor() * SAMPLE_TICK;
        while price <= bar.high {
            // Heavier near the close, so every bar has a point of control away from its edges.
            let weight = 1.0 + 6.0 / (1.0 + (price - bar.close).abs());
            for buy in [true, false] {
                trades.push(Trade {
                    id,
                    time_ms: bar.open_time + 1 + (id as i64 % 3_000_000),
                    price,
                    quantity: weight * (1 + next(9)) as f64,
                    buyer_is_maker: !buy,
                });
                id += 1;
            }
            price += SAMPLE_TICK;
        }
    }
    book.insert(&trades);
    let set = footprint::build_set(
        SetInput {
            bars: &bars,
            bar_ms: 3_600_000,
            candles: None,
            trades: Some(&mut book),
            exchange_tick: Some(SAMPLE_TICK),
            shape: appearance.style.footprint.shape(),
        },
        &mut Cache::default(),
    );
    ProfileFeed {
        footprint: Some(std::sync::Arc::new(set)),
        ..ProfileFeed::default()
    }
}

/// The hours of the sample a TPO preview covers: one UTC day, so one profile fills the card.
const TPO_BARS: usize = 24;

/// The 5-minute candles behind the TPO and session-profile previews: each hourly sample candle cut
/// into twelve that wander from its open to its close inside its range.
fn sample_five_minute_book(bars: &[Kline]) -> CandleBook {
    let mut book = CandleBook::new(300_000, 10_000);
    for bar in bars {
        let range = bar.high - bar.low;
        let mut previous = bar.open;
        for k in 0..12 {
            let along = (k + 1) as f64 / 12.0;
            let wander = (k as f64 * 1.7 + bar.open).sin() * range * 0.22;
            let close =
                (bar.open + (bar.close - bar.open) * along + wander).clamp(bar.low, bar.high);
            // Two of the twelve reach the hour's extremes, so the profile spans the whole range.
            let (mut high, mut low) = (previous.max(close), previous.min(close));
            if k == 3 {
                high = bar.high;
            }
            if k == 8 {
                low = bar.low;
            }
            book.insert([FlowKline {
                kline: Kline {
                    open_time: bar.open_time + k as i64 * 300_000,
                    close_time: bar.open_time + (k as i64 + 1) * 300_000 - 1,
                    open: previous,
                    high,
                    low,
                    close,
                    volume: 1.0 + ((k * 5) % 7) as f64,
                },
                taker_buy: None,
            }]);
            previous = close;
        }
    }
    book
}

/// A deterministic TPO over the first day of the sample: each hourly sample candle is cut into twelve
/// 5-minute candles that wander from its open to its close inside its range, the way a real hour
/// does, and the same builder as a real chart's makes the profile with the user's block size and
/// look. Made-up prices, so it says nothing of any market.
pub fn sample_tpo(appearance: &ChartAppearance) -> ProfileFeed {
    let bars: Vec<Kline> = sample_candles()[..TPO_BARS].to_vec();
    let book = sample_five_minute_book(&bars);
    let mut shape = appearance.style.tpo.shape();
    // The sample is one day: whatever period or custom windows the chart uses, the preview shows it
    // as one profile.
    shape.period = crate::model::chart_type::PeriodKind::Day;
    shape.period_count = 1;
    shape.sessions = crate::model::chart_type::SessionMode::All;
    let set = tpo::build_set(
        tpo::SetInput {
            bars: &bars,
            bar_ms: 3_600_000,
            candles: &book,
            exchange_tick: Some(SAMPLE_TICK),
            shape,
            edits: &[],
            until_ms: i64::MAX,
            now_ms: i64::MAX,
        },
        &mut tpo::Cache::default(),
    );
    ProfileFeed {
        tpo: Some(std::sync::Arc::new(set)),
        ..ProfileFeed::default()
    }
}

/// A deterministic session volume profile over the first day of the sample, from the same made-up
/// 5-minute candles as the TPO preview, with the user's rows, volume and look settings.
pub fn sample_svp(appearance: &ChartAppearance) -> ProfileFeed {
    let bars: Vec<Kline> = sample_candles()[..TPO_BARS].to_vec();
    let book = sample_five_minute_book(&bars);
    let mut shape = appearance.style.svp.shape();
    shape.sessions = crate::model::chart_type::SessionMode::All;
    let set = svp::build_set(
        svp::SetInput {
            bars: &bars,
            bar_ms: 3_600_000,
            candles: &book,
            exchange_tick: Some(SAMPLE_TICK),
            shape,
            until_ms: i64::MAX,
            now_ms: i64::MAX,
        },
        &mut svp::Cache::default(),
    );
    ProfileFeed {
        svp: Some(std::sync::Arc::new(set)),
        ..ProfileFeed::default()
    }
}

/// `construction` with its size replaced by one that suits the sample prices.
fn sized_for_sample(construction: Construction) -> Construction {
    let percent = |percent: f64| SizeRule {
        method: SizeMethod::Percent,
        percent,
        ..SizeRule::default()
    };
    match construction {
        Construction::Renko { source, wicks, .. } => Construction::Renko {
            size: percent(2.4),
            source,
            wicks,
        },
        Construction::Kagi { source, .. } => Construction::Kagi {
            size: percent(2.8),
            source,
        },
        Construction::PointFigure {
            reversal, source, ..
        } => Construction::PointFigure {
            size: percent(1.6),
            reversal,
            source,
        },
        Construction::Range { .. } => Construction::Range { size: percent(2.2) },
        other => other,
    }
}

/// What the preview draws for `chart_type`: the bars (the sample candles, or the constructed ones)
/// and, for a price-based type, the series they came from.
pub fn preview_bars(
    appearance: &ChartAppearance,
    chart_type: ChartType,
) -> (Vec<Kline>, Option<SyntheticSeries>) {
    let sample = sample_candles();
    if chart_type == ChartType::VolumeFootprint {
        let start = sample.len() - FOOTPRINT_BARS;
        return (sample[start..].to_vec(), None);
    }
    if matches!(
        chart_type,
        ChartType::TimePriceOpportunity | ChartType::SessionVolumeProfile
    ) {
        return (sample[..TPO_BARS].to_vec(), None);
    }
    let mut style = appearance.style;
    style.chart_type = chart_type;
    match Construction::of(&style) {
        Some(construction) => {
            let series = build_with(sized_for_sample(construction), &sample, i64::MAX / 2);
            (series.klines(), Some(series))
        }
        None => (sample, None),
    }
}

/// Draws the preview card for `chart_type`.
pub fn show(ui: &mut Ui, appearance: &ChartAppearance, chart_type: ChartType) {
    let width = ui.available_width();
    let card = kit::card_frame(false).show(ui, |ui| {
        ui.set_width(width - 28.0 - 2.0);
        ui.spacing_mut().item_spacing.y = 8.0;
        ui.label(kit::text(
            "Preview",
            kit::size::SUBHEADLINE + 1.0,
            kit::SEMIBOLD,
            theme::text_primary(),
        ));
        let (well, _) = ui.allocate_exact_size(vec2(ui.available_width(), HEIGHT), Sense::hover());
        kit::paint_well(ui, well, 10.0);
        paint(ui, well.shrink(10.0), appearance, chart_type);
    });
    kit::paint_card_highlight(ui, card.response.rect);
}

fn paint(ui: &Ui, plot: Rect, appearance: &ChartAppearance, chart_type: ChartType) {
    let (bars, series) = preview_bars(appearance, chart_type);
    if bars.is_empty() {
        return;
    }
    let (low, high) = bars.iter().fold((f64::MAX, f64::MIN), |(low, high), bar| {
        (low.min(bar.low), high.max(bar.high))
    });
    let padding = ((high - low) * 0.06).max(f64::EPSILON);
    let scale = PriceScale::linear(low - padding, high + padding, plot.top(), plot.height());
    let slot = plot.width() / bars.len() as f32;
    let x_for = |index: usize| plot.left() + slot * (index as f32 + 0.5);
    let painter = ui.painter_at(plot);
    let feed = match chart_type {
        ChartType::VolumeFootprint => Some(sample_footprint(appearance)),
        ChartType::TimePriceOpportunity => Some(sample_tpo(appearance)),
        ChartType::SessionVolumeProfile => Some(sample_svp(appearance)),
        _ => None,
    };
    series_render::paint(&SeriesFrame {
        painter: &painter,
        plot,
        bars: &bars,
        synthetic: series.as_ref().map(|series| series.bars.as_slice()),
        box_size: series.as_ref().and_then(|series| series.size),
        previous_close: None,
        slot_width: slot,
        x_for: &x_for,
        scale: &scale,
        palette: Palette::new(appearance.bullish, appearance.bearish),
        style: &appearance.style,
        chart_type,
        recolor: None,
        price_format: appearance.price_format(),
        profiles: feed.as_ref(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::chart_type::{ChartStyle, ConstructionSource};
    use eframe::egui;

    fn appearance(style: ChartStyle) -> ChartAppearance {
        ChartAppearance {
            style,
            ..ChartAppearance::default()
        }
    }

    #[test]
    fn the_sample_has_rises_falls_and_a_doji() {
        let sample = sample_candles();
        assert!(sample.iter().any(|bar| bar.close > bar.open));
        assert!(sample.iter().any(|bar| bar.close < bar.open));
        assert!(sample.iter().any(|bar| bar.close == bar.open));
        assert!(
            sample
                .windows(2)
                .all(|pair| pair[0].open_time < pair[1].open_time)
        );
        assert!(
            sample
                .iter()
                .map(|bar| bar.volume as i64)
                .collect::<std::collections::HashSet<_>>()
                .len()
                > 4
        );
    }

    #[test]
    fn every_price_based_type_makes_a_readable_figure_whatever_size_the_user_set() {
        for amount in [0.0001, 1.0, 500.0, 1_000_000.0] {
            for kind in ChartType::ALL
                .into_iter()
                .filter(|kind| kind.is_synthetic())
            {
                let mut style = ChartStyle {
                    chart_type: kind,
                    ..ChartStyle::default()
                };
                for size in [
                    &mut style.renko.size,
                    &mut style.kagi.size,
                    &mut style.point_figure.size,
                    &mut style.range.size,
                ] {
                    size.method = SizeMethod::Traditional;
                    size.amount = amount;
                }
                let (bars, series) = preview_bars(&appearance(style), kind);
                assert!(
                    (3..=60).contains(&bars.len()),
                    "{kind:?} with size {amount}: {} bars",
                    bars.len()
                );
                assert!(series.is_some());
            }
        }
    }

    #[test]
    fn the_users_other_construction_settings_are_kept() {
        let mut style = ChartStyle {
            chart_type: ChartType::PointAndFigure,
            ..ChartStyle::default()
        };
        style.point_figure.reversal = 2;
        style.point_figure.source = ConstructionSource::HighLow;
        let kept = sized_for_sample(Construction::of(&style).unwrap());
        assert!(matches!(
            kept,
            Construction::PointFigure {
                reversal: 2,
                source: ConstructionSource::HighLow,
                size: SizeRule {
                    method: SizeMethod::Percent,
                    ..
                },
            }
        ));
    }

    #[test]
    fn standard_types_preview_the_sample_candles_as_they_are() {
        for kind in ChartType::ALL.into_iter().filter(|kind| {
            !kind.is_synthetic()
                && !matches!(
                    kind,
                    ChartType::VolumeFootprint
                        | ChartType::TimePriceOpportunity
                        | ChartType::SessionVolumeProfile
                )
        }) {
            let (bars, series) = preview_bars(&appearance(ChartStyle::default()), kind);
            assert_eq!(bars, sample_candles());
            assert!(series.is_none());
        }
    }

    #[test]
    fn the_footprint_previews_the_last_few_sample_candles_with_a_footprint_of_each() {
        let (bars, series) = preview_bars(
            &appearance(ChartStyle::default()),
            ChartType::VolumeFootprint,
        );
        let sample = sample_candles();
        assert_eq!(bars, sample[sample.len() - FOOTPRINT_BARS..]);
        assert!(series.is_none());
        let feed = sample_footprint(&appearance(ChartStyle::default()));
        let set = feed.footprint.expect("a footprint");
        assert_eq!(
            set.counts.trades, FOOTPRINT_BARS,
            "every bar is built from the sample trades"
        );
        assert!(set.bars.iter().all(Option::is_some));
        // The same every time: the preview must not flicker.
        assert_eq!(
            sample_footprint(&appearance(ChartStyle::default()))
                .footprint
                .unwrap()
                .bars,
            set.bars
        );
    }

    #[test]
    fn the_tpo_previews_one_sample_day_as_one_profile_and_is_the_same_every_time() {
        let appearance = appearance(ChartStyle::default());
        let (bars, series) = preview_bars(&appearance, ChartType::TimePriceOpportunity);
        assert_eq!(bars, sample_candles()[..TPO_BARS]);
        assert!(series.is_none());
        let set = sample_tpo(&appearance).tpo.expect("a profile set");
        assert_eq!(set.profiles.len(), 1);
        let profile = &set.profiles[0];
        assert_eq!(profile.blocks, 48, "a day of 30-minute blocks");
        assert!(profile.rows.len() > 10 && profile.width() > 3);
        assert_eq!(
            sample_tpo(&appearance).tpo.unwrap().profiles[0],
            set.profiles[0]
        );
    }

    #[test]
    fn the_session_profile_previews_one_sample_day_and_is_the_same_every_time() {
        let appearance = appearance(ChartStyle::default());
        let (bars, series) = preview_bars(&appearance, ChartType::SessionVolumeProfile);
        assert_eq!(bars, sample_candles()[..TPO_BARS]);
        assert!(series.is_none());
        let set = sample_svp(&appearance).svp.expect("a profile set");
        assert_eq!(set.profiles.len(), 1);
        let profile = &set.profiles[0];
        assert!(
            profile.rows.len() >= 20 && profile.rows.len() <= 26,
            "about 24 rows: {}",
            profile.rows.len()
        );
        assert!(profile.total() > 0.0 && profile.up() > 0.0 && profile.down() > 0.0);
        assert_eq!(
            sample_svp(&appearance).svp.unwrap().profiles[0],
            set.profiles[0]
        );
    }

    #[test]
    fn every_type_previews_in_a_frame_with_its_colours() {
        let ctx = egui::Context::default();
        for kind in ChartType::ALL {
            let mut style = ChartStyle {
                chart_type: kind,
                ..ChartStyle::default()
            };
            style.candles.up_body = Some([255, 0, 255, 255]);
            let mut shapes = 0;
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                show(ui, &appearance(style), kind);
            });
            shapes += output.shapes.len();
            output.textures_delta.clear();
            // The card, its well and the caption are three shapes; the type itself adds the rest.
            assert!(shapes > 3, "{kind:?} drew {shapes} shapes");
        }
    }
}
