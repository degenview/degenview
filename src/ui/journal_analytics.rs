//! The Journal's Analytics page: risk and consistency figures, the distribution of R, and the
//! breakdown of results by one dimension at a time. A view over the [`Overview`] and a list of
//! [`Slice`]s; it returns the dimension the user picked.

use eframe::egui::{self, Color32, Ui, vec2};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use super::journal_charts::{self as charts, Bar};
use super::journal_kit::{self as jk, Card, Sign};
use super::kit::{self, MEDIUM, REGULAR, SEMIBOLD, size};
use super::paper_grid;
use super::theme;
use degenview_journal::breakdown::{Dimension, Sample, Slice};
use degenview_journal::cells::NONE;
use degenview_journal::edge::{ComponentKey, EdgeScore};
use degenview_journal::format;
use degenview_journal::metric::Metric;
use degenview_journal::overview::Overview;
use degenview_journal::process::{MIN_REVIEWED, ProcessScore};

const CHART_HEIGHT: f32 = 220.0;
const WIDE: f32 = 980.0;

fn dec_tone(value: Decimal) -> Color32 {
    Sign::of(value.to_f64().unwrap_or(0.0)).text()
}

/// One figure inside a card: a quiet well with its label over its value.
fn tile(ui: &mut Ui, label: &str, value: &str, tip: &str, width: f32) {
    egui::Frame::new()
        .fill(theme::ink(6))
        .corner_radius(8.0)
        .inner_margin(egui::Margin::symmetric(12, 9))
        .show(ui, |ui| {
            ui.set_width(width - 24.0);
            ui.vertical(|ui| {
                jk::label(ui, label);
                ui.label(kit::text(
                    value,
                    size::BODY + 1.0,
                    MEDIUM,
                    theme::text_primary(),
                ));
            });
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
    process: &ProcessScore,
    dimension: Dimension,
    slices: &[Slice],
    currency: &str,
) -> Option<Dimension> {
    let mut picked = None;
    let s = &overview.summary;
    let wide = ui.available_width() >= WIDE;

    // The two scores, side by side when there is room. They never blend.
    let scores: [&dyn Fn(&mut Ui); 2] = [&|ui| edge_card(ui, &overview.edge), &|ui| {
        process_card(ui, process)
    }];
    if wide {
        let width = (ui.available_width() - jk::GAP) / 2.0;
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = jk::GAP;
            for score in scores {
                ui.allocate_ui_with_layout(
                    vec2(width, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_width(width);
                        score(ui);
                    },
                );
            }
        });
    } else {
        for score in scores {
            score(ui);
            ui.add_space(jk::GAP);
        }
    }
    ui.add_space(jk::GAP);

    // Risk and consistency.
    let risk = Card::new("Risk and consistency").subtitle(
        "Risk-adjusted ratios use daily returns on trading days, are not annualised, and appear only with enough data.",
    );
    jk::card(
        ui,
        &risk,
        |_| {},
        |ui| {
            let dd_pct = s
                .drawdown
                .max_pct
                .map_or(String::new(), |p| format!(" ({})", format::percent(p)));
            let tiles: Vec<(&str, String, &str)> = vec![
                (
                    "Max drawdown",
                    format!("{}{dd_pct}", format::money(s.drawdown.max)),
                    "Deepest fall of realised equity below its running peak.",
                ),
                (
                    "Avg drawdown",
                    match &s.drawdown.avg_episode {
                        Metric::Value(v) => format::money(*v),
                        Metric::Unavailable(_) => NONE.to_owned(),
                    },
                    "Mean of each drawdown episode's deepest point.",
                ),
                (
                    "Drawdown episodes",
                    s.drawdown.episodes.to_string(),
                    "Separate falls below a peak until it was regained.",
                ),
                (
                    "Current drawdown",
                    format::money(s.drawdown.current),
                    "Distance below the peak right now.",
                ),
                (
                    "Sharpe (daily)",
                    metric_f(&s.sharpe, format::ratio),
                    "Mean daily return over its standard deviation. Needs an initial balance and 30 trading days.",
                ),
                (
                    "Sortino (daily)",
                    metric_f(&s.sortino, format::ratio),
                    "Mean daily return over downside deviation. Needs an initial balance, 30 trading days and a losing day.",
                ),
                (
                    "Calmar",
                    metric_f(&s.calmar, format::ratio),
                    "Return on the initial balance over max drawdown %. Needs both.",
                ),
                (
                    "SQN",
                    metric_f(&s.sqn, format::ratio),
                    "sqrt(n) x mean R / stdev R. Needs 30 trades with an R.",
                ),
                (
                    "Average R",
                    metric_f(&s.r.average, format::r_multiple),
                    "Mean realised R over trades with a stop.",
                ),
                (
                    "Best R",
                    metric_f(&s.r.best, format::r_multiple),
                    "The best realised R.",
                ),
                (
                    "Worst R",
                    metric_f(&s.r.worst, format::r_multiple),
                    "The worst realised R.",
                ),
                (
                    "Trades with R",
                    s.r.count.to_string(),
                    "Closed trades that have a stop, attached to the order or entered in the review.",
                ),
                (
                    "Longest win streak",
                    s.streaks.max_wins.to_string(),
                    "Most winning trades in a row, in close order.",
                ),
                (
                    "Longest loss streak",
                    s.streaks.max_losses.to_string(),
                    "Most losing trades in a row.",
                ),
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
            ui.add_space(6.0);
            ui.label(kit::text(
                "Distribution of R",
                size::BODY,
                SEMIBOLD,
                theme::text_primary(),
            ));
            ui.add_space(4.0);
            let bars: Vec<Bar> = overview
                .r_histogram
                .bins
                .iter()
                .map(|bin| Bar {
                    label: format!("{:+.1}R", bin.low),
                    value: bin.count as f64,
                    color: if bin.high <= 0.0 {
                        Sign::Loss.data()
                    } else {
                        Sign::Gain.data()
                    },
                    detail: format!(
                        "{:+.1}R to {:+.1}R \u{00b7} {} trade{}",
                        bin.low,
                        bin.high,
                        bin.count,
                        if bin.count == 1 { "" } else { "s" }
                    ),
                })
                .collect();
            charts::bar_chart(
                ui,
                &bars,
                &|v| format!("{v:.0}"),
                150.0,
                "No trades with a stop yet. Add a stop in a trade's review to see R.",
            );
        },
    );
    ui.add_space(12.0);

    // Breakdown.
    jk::card(
        ui,
        &Card::new("Breakdown"),
        |_| {},
        |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                for candidate in Dimension::ALL {
                    if kit::value_chip(ui, candidate.label(), candidate == dimension).clicked() {
                        picked = Some(candidate);
                    }
                }
            });
            ui.add_space(6.0);
            ui.label(kit::text(
                dimension.description(),
                size::CAPTION + 1.0,
                REGULAR,
                theme::text_tertiary(),
            ));
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
                        color: if value >= 0.0 {
                            Sign::Gain.data()
                        } else {
                            Sign::Loss.data()
                        },
                        detail: format!(
                            "{} {currency} \u{00b7} {} trade{}{}",
                            format::signed_money(slice.net),
                            slice.trades,
                            if slice.trades == 1 { "" } else { "s" },
                            slice
                                .sample
                                .note()
                                .map_or(String::new(), |n| format!(" \u{00b7} {n}"))
                        ),
                    }
                })
                .collect();
            charts::bar_chart(
                ui,
                &bars,
                &|v| format::compact(v),
                CHART_HEIGHT,
                "Nothing to show: no closed trades in this selection",
            );
            ui.add_space(10.0);
            slice_table(ui, slices, currency);
        },
    );
    picked
}

fn component_input(key: ComponentKey, input: Option<f64>) -> String {
    let Some(value) = input else {
        return NONE.to_owned();
    };
    match key {
        ComponentKey::WinRate | ComponentKey::Drawdown | ComponentKey::Consistency => {
            format::percent(value)
        }
        ComponentKey::ProfitFactor | ComponentKey::AvgWinLoss | ComponentKey::Recovery => {
            format::ratio(value)
        }
    }
}

fn edge_card(ui: &mut Ui, edge: &EdgeScore) {
    let subtitle = format!(
        "Version {}, LuxAlgo's published formula. Results only.",
        edge.version
    );
    let spec = Card::new("Edge score").subtitle(&subtitle);
    let score = match &edge.score {
        Metric::Value(score) => format!("{score:.0} / 100"),
        Metric::Unavailable(reason) => format!("withheld: {}", reason.explain()),
    };
    jk::card(
        ui,
        &spec,
        |ui| {
            ui.label(kit::text(
                score,
                size::BODY + 2.0,
                SEMIBOLD,
                theme::text_primary(),
            ));
        },
        |ui| edge_rows(ui, edge),
    );
}

fn edge_rows(ui: &mut Ui, edge: &EdgeScore) {
    for component in &edge.components {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            charts::fixed_cell(ui, vec2(120.0, 22.0), |ui| {
                ui.label(kit::text(
                    component.key.label(),
                    size::BODY,
                    REGULAR,
                    theme::text_primary(),
                ));
            });
            charts::fixed_cell(ui, vec2(48.0, 22.0), |ui| {
                ui.label(kit::text(
                    format!("{:.0}", component.value),
                    size::BODY,
                    MEDIUM,
                    theme::text_primary(),
                ));
            });
            charts::fixed_cell(ui, vec2(52.0, 22.0), |ui| {
                ui.label(kit::text(
                    format!("w {}", component.weight),
                    size::CAPTION + 1.0,
                    REGULAR,
                    theme::text_tertiary(),
                ));
            });
            charts::fixed_cell(ui, vec2(70.0, 22.0), |ui| {
                ui.label(kit::text(
                    component_input(component.key, component.input),
                    size::CAPTION + 1.0,
                    REGULAR,
                    theme::text_secondary(),
                ));
            });
        })
        .response
        .on_hover_text(format!(
            "Full marks: {}.{}",
            component.key.full_marks(),
            component
                .note
                .map_or(String::new(), |note| format!(" {note}"))
        ));
    }
    ui.add_space(6.0);
    ui.label(kit::text(
        "Hover a row for its full-marks threshold. The score measures the record, not the future; the trade count matters.",
        size::CAPTION,
        REGULAR,
        theme::text_tertiary(),
    ));
}

fn process_card(ui: &mut Ui, process: &ProcessScore) {
    let spec =
        Card::new("Process score").subtitle("From your own reviews. Never part of the Edge score.");
    let score = match &process.score {
        Metric::Value(score) => format!("{score:.0} / 100"),
        Metric::Unavailable(_) => format!("needs {MIN_REVIEWED} reviewed trades"),
    };
    jk::card(
        ui,
        &spec,
        |ui| {
            ui.label(kit::text(
                score,
                size::BODY + 2.0,
                SEMIBOLD,
                theme::text_primary(),
            ));
        },
        |ui| process_rows(ui, process),
    );
}

fn process_rows(ui: &mut Ui, process: &ProcessScore) {
    for component in &process.components {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            charts::fixed_cell(ui, vec2(130.0, 22.0), |ui| {
                ui.label(kit::text(
                    component.key.label(),
                    size::BODY,
                    REGULAR,
                    theme::text_primary(),
                ));
            });
            charts::fixed_cell(ui, vec2(70.0, 22.0), |ui| {
                let text = component
                    .value
                    .map_or(NONE.to_owned(), |v| format!("{v:.0}"));
                ui.label(kit::text(text, size::BODY, MEDIUM, theme::text_primary()));
            });
            charts::fixed_cell(ui, vec2(120.0, 22.0), |ui| {
                ui.label(kit::text(
                    format!(
                        "w {} \u{00b7} {} trades",
                        component.weight, component.samples
                    ),
                    size::CAPTION + 1.0,
                    REGULAR,
                    theme::text_tertiary(),
                ));
            });
        })
        .response
        .on_hover_text(component.key.definition());
    }
    ui.add_space(6.0);
    ui.label(kit::text(
        format!(
            "{} of {} closed trades reviewed. Ratings are self-reported: a record of process, not a measure of skill.",
            process.reviewed, process.closed
        ),
        size::CAPTION,
        REGULAR,
        theme::text_tertiary(),
    ));
}

fn sample_badge(ui: &mut Ui, sample: Sample) {
    match sample {
        Sample::Thin => {
            jk::pill(ui, "THIN", kit::Tone::Warning);
        }
        Sample::Low => {
            jk::pill(ui, "SMALL", kit::Tone::Neutral);
        }
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
    use degenview_journal::breakdown::{Context, breakdown};
    use degenview_journal::calendar::utc;
    use degenview_journal::model::{CostBasis, Side, Trade};
    use degenview_journal::overview::{Input, compute};
    use degenview_journal::reconstruct::reconstruct;
    use degenview_journal::testing::exec;
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
        let process = degenview_journal::process::process_score(&refs, &reviews, &[]);
        let ctx = egui::Context::default();
        for dimension in Dimension::ALL {
            for width in [1300.0, 640.0] {
                let slices = breakdown(
                    &refs,
                    dimension,
                    &Context {
                        reviews: &reviews,
                        playbook_names: &HashMap::new(),
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
                            assert_eq!(
                                show(ui, &overview, &process, dimension, &slices, "USD"),
                                None
                            );
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
            show(ui, &empty, &process, Dimension::Weekday, &[], "");
        });
        out.textures_delta.clear();
    }
}
