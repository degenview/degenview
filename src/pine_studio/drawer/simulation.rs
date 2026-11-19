//! The Report tab's Simulation card: the closed trades redrawn at random many times over
//! (`pine::strategy::monte_carlo`), as a few figures and a fan of the equity paths they make.

use super::tab_chrome::{self as chrome, card};
use crate::pine::strategy::StrategyReport;
use crate::pine::strategy::monte_carlo::{MIN_TRADES, MonteCarlo, Spread};
use crate::ui::pine::report::{PineReportFormat, metric_cells};
use crate::ui::theme;
use eframe::egui::{self, Align2, FontId, Margin, RichText, Shape, Stroke, Ui, pos2, vec2};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

const FAN_HEIGHT: f32 = 72.0;

/// What a frame remembers of the last resampling: which trades it was for, and its answer.
#[derive(Clone)]
struct Remembered {
    trades: u64,
    result: Option<Arc<MonteCarlo>>,
}

/// A hash of what the resampling reads: the capital and each trade's profit.
fn trades_fingerprint(report: &StrategyReport) -> u64 {
    let mut hasher = DefaultHasher::new();
    report.settings.initial_capital.to_bits().hash(&mut hasher);
    for trade in &report.trades {
        trade.profit.to_bits().hash(&mut hasher);
    }
    hasher.finish()
}

/// The resampling of `report`, redone only when its trades change; a live tick that closes no trade
/// reuses the last answer.
fn resampled(ui: &Ui, report: &StrategyReport) -> Option<Arc<MonteCarlo>> {
    let id = ui.make_persistent_id("pine-report-monte-carlo");
    let trades = trades_fingerprint(report);
    if let Some(known) = ui
        .data(|data| data.get_temp::<Remembered>(id))
        .filter(|known| known.trades == trades)
    {
        return known.result;
    }
    let result = MonteCarlo::run(report).map(Arc::new);
    ui.data_mut(|data| {
        data.insert_temp(
            id,
            Remembered {
                trades,
                result: result.clone(),
            },
        )
    });
    result
}

/// The card under the figures. With fewer than [`MIN_TRADES`] closed trades it says so instead.
pub(super) fn simulation_card(ui: &mut Ui, report: &StrategyReport) {
    chrome::caption(ui, "Simulation");
    let padded = Margin::symmetric(12, 10);
    let Some(result) = resampled(ui, report) else {
        card(ui, |ui| {
            egui::Frame::new().inner_margin(padded).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    RichText::new(format!(
                        "Needs {MIN_TRADES} closed trades to reshuffle; {} so far.",
                        report.trades.len()
                    ))
                    .size(10.0)
                    .color(theme::text_tertiary()),
                );
            });
        });
        return;
    };
    card(ui, |ui| {
        egui::Frame::new().inner_margin(padded).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 10.0;
            ui.label(
                RichText::new(format!(
                    "{} reshuffles of the {} closed trades, drawn with replacement. Profits are added as money, so position sizing does not compound.",
                    result.runs, result.trades
                ))
                .size(9.0)
                .color(theme::text_secondary()),
            );
            figures(ui, &result);
            fan_chart(ui, &result);
        });
    });
}

fn figures(ui: &mut Ui, result: &MonteCarlo) {
    let money = PineReportFormat::money;
    let drawdown = |fraction| PineReportFormat::percent(fraction, false);
    let ending = result.final_equity;
    metric_cells(
        ui,
        "pine-report-simulation",
        [
            (
                "Final equity, median",
                money(ending.median),
                ending.median - result.initial_capital,
            ),
            (
                "Final equity, 5th to 95th",
                format!("{} to {}", money(ending.low), money(ending.high)),
                0.0,
            ),
            (
                "Max drawdown, median",
                drawdown(result.max_drawdown.median),
                0.0,
            ),
            (
                "Max drawdown, worst 5%",
                drawdown(result.max_drawdown.high),
                0.0,
            ),
            (
                "Chance of ending in loss",
                PineReportFormat::percent(result.loss_probability, false),
                0.0,
            ),
        ],
        theme::text_primary(),
    );
}

/// The 5th to 95th percentile of equity along the trades as a band, the median through it, and the
/// initial capital as a dashed line.
fn fan_chart(ui: &mut Ui, result: &MonteCarlo) {
    let (rect, _) =
        ui.allocate_exact_size(vec2(ui.available_width(), FAN_HEIGHT), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    let weak = theme::text_secondary();
    let low = result
        .fan
        .iter()
        .map(|step| step.low)
        .fold(result.initial_capital, f64::min);
    let high = result
        .fan
        .iter()
        .map(|step| step.high)
        .fold(result.initial_capital, f64::max);
    let span = (high - low).max(1e-9);
    let inset = 2.0;
    let usable = rect.height() - 2.0 * inset;
    let y = |value: f64| rect.max.y - ((value - low) / span) as f32 * usable - inset;
    let last = (result.fan.len() - 1) as f32;
    let x = |index: usize| rect.min.x + rect.width() * index as f32 / last;

    let baseline = y(result.initial_capital);
    painter.extend(Shape::dashed_line(
        &[pos2(rect.min.x, baseline), pos2(rect.max.x, baseline)],
        Stroke::new(1.0, theme::with_alpha(weak, 0.5)),
        3.0,
        3.0,
    ));

    let tone = if result.final_equity.median >= result.initial_capital {
        theme::WIN
    } else {
        theme::LOSS
    };
    let band = theme::with_alpha(tone, 0.18);
    for (index, pair) in result.fan.windows(2).enumerate() {
        let [from, to]: [&Spread; 2] = [&pair[0], &pair[1]];
        painter.add(Shape::convex_polygon(
            vec![
                pos2(x(index), y(from.high)),
                pos2(x(index + 1), y(to.high)),
                pos2(x(index + 1), y(to.low)),
                pos2(x(index), y(from.low)),
            ],
            band,
            Stroke::NONE,
        ));
    }
    let median = result
        .fan
        .iter()
        .enumerate()
        .map(|(index, step)| pos2(x(index), y(step.median)))
        .collect();
    painter.add(Shape::line(median, Stroke::new(1.5, tone)));
    painter.text(
        rect.min + vec2(4.0, 4.0),
        Align2::LEFT_TOP,
        "Equity, 5th to 95th percentile",
        FontId::proportional(9.0),
        weak,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pine::strategy::{StrategySettings, Trade};

    fn report(profits: &[f64]) -> StrategyReport {
        StrategyReport {
            settings: StrategySettings::default(),
            trades: profits
                .iter()
                .map(|&profit| Trade {
                    profit,
                    ..Default::default()
                })
                .collect(),
            open_trades: Vec::new(),
            equity: Vec::new(),
            open_profit: 0.0,
            chart: crate::model::ChartKind::Standard,
        }
    }

    #[test]
    fn the_fingerprint_follows_the_profits_and_the_capital() {
        let base = report(&[1.0, -2.0, 3.0]);
        assert_eq!(trades_fingerprint(&base), trades_fingerprint(&base.clone()));
        assert_ne!(
            trades_fingerprint(&base),
            trades_fingerprint(&report(&[1.0, -2.0, 4.0]))
        );
        let mut richer = base.clone();
        richer.settings.initial_capital += 1.0;
        assert_ne!(trades_fingerprint(&base), trades_fingerprint(&richer));
    }

    #[test]
    fn the_card_draws_with_too_few_trades_and_with_enough() {
        let few = report(&[5.0, -3.0]);
        let many = report(&[5.0, -3.0, 8.0, -1.0, 4.0, -6.0, 9.0, -2.0, 7.0, -4.0, 3.0]);
        egui::__run_test_ui(|ui| {
            simulation_card(ui, &few);
            simulation_card(ui, &many);
        });
    }
}
