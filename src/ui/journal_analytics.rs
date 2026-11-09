//! The Journal's Analytics page: risk and consistency figures, the distribution of R, and the
//! breakdown of results by one dimension at a time. A view over the [`Overview`] and a list of
//! [`Slice`]s; it returns the dimension the user picked.

use eframe::egui::{self, Color32, Ui, vec2};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use super::journal_charts::{self as charts, Bar};
use super::kit::{self, MEDIUM, REGULAR, SEMIBOLD, size};
use super::paper_grid;
use super::theme;
use crate::model::journal::breakdown::{Dimension, Sample, Slice};
use crate::model::journal::cells::NONE;
use crate::model::journal::format;
use crate::model::journal::metric::Metric;
use crate::model::journal::overview::Overview;

const CHART_HEIGHT: f32 = 220.0;
const WIDE: f32 = 980.0;

fn tone(value: f64) -> Color32 {
    if value > 0.0 {
        theme::WIN
    } else if value < 0.0 {
        theme::LOSS
    } else {
        theme::text_primary()
    }
}

fn dec_tone(value: Decimal) -> Color32 {
    tone(value.to_f64().unwrap_or(0.0))
}

fn tile(ui: &mut Ui, label: &str, value: &str, tip: &str, width: f32) {
    egui::Frame::new()
        .fill(theme::quaternary(0.3))
        .corner_radius(10.0)
        .inner_margin(egui::Margin::symmetric(12, 9))
        .show(ui, |ui| {
            ui.set_width(width - 24.0);
            ui.label(kit::text(
                label.to_uppercase(),
                size::CAPTION,
                SEMIBOLD,
                theme::text_tertiary(),
            ));
            ui.label(kit::text(
                value,
                size::BODY + 1.0,
                MEDIUM,
                theme::text_primary(),
            ));
        })
        .response
        .on_hover_text(tip);
}

fn metric_f(metric: &Metric<f64>, text: impl Fn(f64) -> String) -> String {
    match metric {
        Metric::Value(v) => text(*v),
        Metric::Unavailable(reason) => format!("{NONE}  ({})", reason.explain()),
    }
}

/// Returns the dimension picked, if the user changed it.
pub fn show(
    ui: &mut Ui,
    overview: &Overview,
    dimension: Dimension,
    slices: &[Slice],
    currency: &str,
) -> Option<Dimension> {
    let mut picked = None;
    let s = &overview.summary;
    let wide = ui.available_width() >= WIDE;

    // Risk and consistency.
    kit::card_frame(false).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(kit::text("Risk and consistency", size::BODY + 1.0, SEMIBOLD, theme::text_primary()));
        ui.label(kit::text(
            "Risk-adjusted ratios use daily returns on trading days, are not annualised, and appear only with enough data.",
            size::CAPTION + 1.0,
            REGULAR,
            theme::text_tertiary(),
        ));
        ui.add_space(10.0);
        let dd_pct = s.drawdown.max_pct.map_or(String::new(), |p| format!(" ({})", format::percent(p)));
        let tiles: Vec<(&str, String, &str)> = vec![
            ("Max drawdown", format!("{}{dd_pct}", format::money(s.drawdown.max)), "Deepest fall of realised equity below its running peak."),
            (
                "Avg drawdown",
                match &s.drawdown.avg_episode {
                    Metric::Value(v) => format::money(*v),
                    Metric::Unavailable(_) => NONE.to_owned(),
                },
                "Mean of each drawdown episode's deepest point.",
            ),
            ("Drawdown episodes", s.drawdown.episodes.to_string(), "Separate falls below a peak until it was regained."),
            ("Current drawdown", format::money(s.drawdown.current), "Distance below the peak right now."),
            ("Sharpe (daily)", metric_f(&s.sharpe, format::ratio), "Mean daily return over its standard deviation. Needs an initial balance and 30 trading days."),
            ("Sortino (daily)", metric_f(&s.sortino, format::ratio), "Mean daily return over downside deviation. Needs an initial balance, 30 trading days and a losing day."),
            ("Calmar", metric_f(&s.calmar, format::ratio), "Return on the initial balance over max drawdown %. Needs both."),
            ("SQN", metric_f(&s.sqn, format::ratio), "sqrt(n) x mean R / stdev R. Needs 30 trades with an R."),
            ("Average R", metric_f(&s.r.average, format::r_multiple), "Mean realised R over trades with a stop."),
            ("Best R", metric_f(&s.r.best, format::r_multiple), "The best realised R."),
            ("Worst R", metric_f(&s.r.worst, format::r_multiple), "The worst realised R."),
            ("Trades with R", s.r.count.to_string(), "Closed trades that have a stop, attached to the order or entered in the review."),
            ("Longest win streak", s.streaks.max_wins.to_string(), "Most winning trades in a row, in close order."),
            ("Longest loss streak", s.streaks.max_losses.to_string(), "Most losing trades in a row."),
            (
                "Avg winner held",
                match &s.durations.average_win_ms {
                    Metric::Value(ms) => format::duration(*ms),
                    Metric::Unavailable(_) => NONE.to_owned(),
                },
                "Mean time from open to close of winning trades.",
            ),
            (
                "Avg loser held",
                match &s.durations.average_loss_ms {
                    Metric::Value(ms) => format::duration(*ms),
                    Metric::Unavailable(_) => NONE.to_owned(),
                },
                "Mean time from open to close of losing trades.",
            ),
        ];
        let per_row = if wide { 4 } else { 2 };
        let gap = 10.0;
        let width = (ui.available_width() - gap * (per_row as f32 - 1.0)) / per_row as f32;
        for chunk in tiles.chunks(per_row) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for (label, value, tip) in chunk {
                    tile(ui, label, value, tip, width);
                }
            });
            ui.add_space(gap);
        }
        ui.add_space(4.0);
        ui.label(kit::text("Distribution of R", size::BODY, SEMIBOLD, theme::text_primary()));
        ui.add_space(4.0);
        let bars: Vec<Bar> = overview
            .r_histogram
            .bins
            .iter()
            .map(|bin| Bar {
                label: format!("{:+.1}R", bin.low),
                value: bin.count as f64,
                color: if bin.high <= 0.0 { theme::LOSS } else { theme::WIN },
                detail: format!("{:+.1}R to {:+.1}R \u{00b7} {} trade{}", bin.low, bin.high, bin.count, if bin.count == 1 { "" } else { "s" }),
            })
            .collect();
        charts::bar_chart(ui, &bars, &|v| format!("{v:.0}"), 150.0, "No trades with a stop yet. Add a stop in a trade's review to see R.");
    });
    ui.add_space(12.0);

    // Breakdown.
    kit::card_frame(false).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(kit::text("Breakdown", size::BODY + 1.0, SEMIBOLD, theme::text_primary()));
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            for candidate in Dimension::ALL {
                if kit::value_chip(ui, candidate.label(), candidate == dimension).clicked() {
                    picked = Some(candidate);
                }
            }
        });
        ui.add_space(6.0);
        ui.label(kit::text(dimension.description(), size::CAPTION + 1.0, REGULAR, theme::text_tertiary()));
        if dimension.overlapping() {
            ui.label(kit::text(
                "These slices overlap, so their trade counts add up to more than the number of trades. Associations are descriptive, not causes.",
                size::CAPTION + 1.0,
                REGULAR,
                theme::text_tertiary(),
            ));
        }
        ui.add_space(10.0);
        let bars: Vec<Bar> = slices
            .iter()
            .map(|slice| {
                let value = slice.net.to_f64().unwrap_or(0.0);
                Bar {
                    label: slice.label.clone(),
                    value,
                    color: if value >= 0.0 { theme::WIN } else { theme::LOSS },
                    detail: format!(
                        "{} {currency} \u{00b7} {} trade{}{}",
                        format::signed_money(slice.net),
                        slice.trades,
                        if slice.trades == 1 { "" } else { "s" },
                        slice.sample.note().map_or(String::new(), |n| format!(" \u{00b7} {n}"))
                    ),
                }
            })
            .collect();
        charts::bar_chart(ui, &bars, &|v| format::compact(v), CHART_HEIGHT, "Nothing to show: no closed trades in this selection");
        ui.add_space(10.0);
        slice_table(ui, slices, currency);
    });
    picked
}

fn sample_badge(ui: &mut Ui, sample: Sample) {
    match sample {
        Sample::Thin => paper_grid::tag(ui, "THIN", theme::LOSS),
        Sample::Low => paper_grid::tag(ui, "SMALL", theme::text_secondary()),
        Sample::Enough => {}
    }
}

fn slice_table(ui: &mut Ui, slices: &[Slice], currency: &str) {
    if slices.is_empty() {
        return;
    }
    let widths = [1.6, 0.7, 0.9, 0.9, 0.9, 1.2, 0.8];
    let heads = [
        "Slice",
        "Trades",
        "Win rate",
        "Profit factor",
        "Avg R",
        "Net P&L",
        "",
    ];
    let total: f32 = widths.iter().sum();
    let available = ui.available_width();
    let col = |index: usize| available * widths[index] / total;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for (index, head) in heads.iter().enumerate() {
            charts::fixed_cell(ui, vec2(col(index), 20.0), |ui| {
                ui.label(kit::text(
                    head.to_uppercase(),
                    size::CAPTION,
                    SEMIBOLD,
                    theme::text_tertiary(),
                ));
            });
        }
    });
    for slice in slices {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let cells: [String; 6] = [
                slice.label.clone(),
                slice.trades.to_string(),
                match &slice.win_rate {
                    Metric::Value(v) => format::percent(*v),
                    Metric::Unavailable(_) => NONE.to_owned(),
                },
                match &slice.profit_factor {
                    Metric::Value(v) => format::ratio(*v),
                    Metric::Unavailable(_) => NONE.to_owned(),
                },
                match &slice.average_r {
                    Metric::Value(v) => format!("{} ({})", format::r_multiple(*v), slice.r_count),
                    Metric::Unavailable(_) => NONE.to_owned(),
                },
                format!("{} {currency}", format::signed_money(slice.net))
                    .trim()
                    .to_owned(),
            ];
            for (index, text) in cells.iter().enumerate() {
                charts::fixed_cell(ui, vec2(col(index), 26.0), |ui| {
                    let color = if index == 5 {
                        dec_tone(slice.net)
                    } else {
                        theme::text_primary()
                    };
                    paper_grid::text_cell(ui, text, color);
                });
            }
            charts::fixed_cell(ui, vec2(col(6), 26.0), |ui| {
                sample_badge(ui, slice.sample);
            });
        });
    }
    ui.add_space(4.0);
    ui.label(kit::text(
        "THIN: fewer than 5 trades. SMALL: fewer than 30. Neither is evidence of a real difference.",
        size::CAPTION,
        REGULAR,
        theme::text_tertiary(),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::journal::breakdown::{Context, breakdown};
    use crate::model::journal::calendar::utc;
    use crate::model::journal::model::{CostBasis, Side, Trade};
    use crate::model::journal::overview::{Input, compute};
    use crate::model::journal::reconstruct::reconstruct;
    use crate::model::journal::testing::exec;
    use std::collections::HashMap;

    #[test]
    fn the_page_draws_for_every_dimension_with_and_without_data() {
        let mut e = Vec::new();
        for i in 0..12i64 {
            let exit = if i % 3 == 0 { "90" } else { "112" };
            e.push(
                exec(
                    &format!("{i}a"),
                    Side::Buy,
                    "1",
                    "100",
                    (i + 1) * 86_400_000,
                )
                .stop("95"),
            );
            e.push(exec(
                &format!("{i}b"),
                Side::Sell,
                "1",
                exit,
                (i + 1) * 86_400_000 + 1_000,
            ));
        }
        let trades = reconstruct(&e, |_| CostBasis::AverageCost);
        let refs: Vec<&Trade> = trades.iter().collect();
        let reviews = HashMap::new();
        let overview = compute(&Input {
            trades: &refs,
            previous: None,
            before: &[],
            accounts: &[],
            from_ms: None,
            to_ms: None,
            currency: None,
            reviews: &reviews,
            breakeven_tolerance: Decimal::ZERO,
            offset: &utc,
        });
        let ctx = egui::Context::default();
        for dimension in Dimension::ALL {
            for width in [1300.0, 640.0] {
                let slices = breakdown(
                    &refs,
                    dimension,
                    &Context {
                        reviews: &reviews,
                        offset: &utc,
                        breakeven_tolerance: Decimal::ZERO,
                    },
                );
                let mut out = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::pos2(0.0, 0.0),
                            vec2(width, 2000.0),
                        )),
                        ..Default::default()
                    },
                    |ui| {
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            assert_eq!(show(ui, &overview, dimension, &slices, "USD"), None);
                        });
                    },
                );
                out.textures_delta.clear();
            }
        }
        let empty = compute(&Input {
            trades: &[],
            previous: None,
            before: &[],
            accounts: &[],
            from_ms: None,
            to_ms: None,
            currency: None,
            reviews: &reviews,
            breakeven_tolerance: Decimal::ZERO,
            offset: &utc,
        });
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            show(ui, &empty, Dimension::Weekday, &[], "");
        });
        out.textures_delta.clear();
    }
}
