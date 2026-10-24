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
        for kind in ChartType::ALL
            .into_iter()
            .filter(|kind| !kind.is_synthetic())
        {
            let (bars, series) = preview_bars(&appearance(ChartStyle::default()), kind);
            assert_eq!(bars, sample_candles());
            assert!(series.is_none());
        }
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
