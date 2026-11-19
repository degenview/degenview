//! The Report tab's Simulation card: the closed trades redrawn at random many times over
//! (`pine::strategy::monte_carlo`), as a few figures and a fan of the equity paths they make, with
//! a legend saying what the band and the lines are.

use super::figures::{BLOCK_GAP, Emphasis, Figure, grid};
use super::plot::{self, Plot};
use super::tab_chrome::{self as chrome, panel};
use crate::pine::strategy::StrategyReport;
use crate::pine::strategy::monte_carlo::{MAX_TRADES, MonteCarlo, Spread, Unavailable};
use crate::ui::pine::report::{self, PineReportFormat};
use crate::ui::{kit, theme};
use eframe::egui::{Color32, Rect, Sense, Shape, Stroke, Ui, Vec2, pos2, vec2};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

const FAN_HEIGHT: f32 = 96.0;
/// How strongly the band of the middle 90% is filled.
const BAND_ALPHA: f32 = 0.18;

/// What a frame remembers of the last resampling: which trades it was for, and its answer.
#[derive(Clone)]
struct Remembered {
    trades: u64,
    result: Result<Arc<MonteCarlo>, Unavailable>,
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
fn resampled(ui: &Ui, report: &StrategyReport) -> Result<Arc<MonteCarlo>, Unavailable> {
    let id = ui.make_persistent_id("pine-report-monte-carlo");
    let trades = trades_fingerprint(report);
    if let Some(known) = ui
        .data(|data| data.get_temp::<Remembered>(id))
        .filter(|known| known.trades == trades)
    {
        return known.result;
    }
    let result = MonteCarlo::run(report).map(Arc::new);
    // Cached by the trades it read, so a refusal is kept as well as an answer.
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

/// The card under the equity. With too few or too many closed trades it says why there is none.
pub(super) fn simulation_card(ui: &mut Ui, report: &StrategyReport) {
    chrome::caption(ui, "Simulation");
    let result = match resampled(ui, report) {
        Ok(result) => result,
        Err(unavailable) => {
            unavailable_note(ui, unavailable);
            return;
        }
    };
    panel(ui, |ui| {
        ui.label(kit::text(
            format!(
                "{} runs, each drawing {} of the closed trades at random, repeats allowed. Profits add up as money, so position sizing does not compound.",
                PineReportFormat::count(result.runs),
                result.trades
            ),
            kit::size::CAPTION,
            kit::REGULAR,
            theme::text_secondary(),
        ));
        ui.add_space(BLOCK_GAP);
        let key = key_figures(&result);
        grid(ui, &key, key.len(), Emphasis::Key);
        ui.add_space(BLOCK_GAP);
        grid(ui, &quiet_figures(&result), key.len(), Emphasis::Quiet);
        ui.add_space(BLOCK_GAP);
        chrome::rule(ui);
        ui.add_space(BLOCK_GAP);
        fan_chart(ui, &result);
        ui.add_space(8.0);
        legend(
            ui,
            plot::tone(result.final_equity.median, result.initial_capital),
        );
    });
}

/// What the card says when there is nothing to draw: why, and what the simulation does.
fn unavailable_note(ui: &mut Ui, unavailable: Unavailable) {
    let what_it_does = match unavailable {
        Unavailable::TooFew { .. } => {
            "Draws the trades at random, repeats allowed, to show how far the result could move if the wins and losses had come out differently.".to_owned()
        }
        Unavailable::TooMany { have } => format!(
            "The strategy has closed {}; the simulation takes up to {} so the tab stays quick.",
            PineReportFormat::count(have),
            PineReportFormat::count(MAX_TRADES)
        ),
    };
    panel(ui, |ui| {
        ui.label(kit::text(
            unavailable.to_string(),
            kit::size::CALLOUT,
            kit::MEDIUM,
            theme::text_secondary(),
        ));
        ui.add_space(3.0);
        ui.label(kit::text(
            what_it_does,
            kit::size::CAPTION,
            kit::REGULAR,
            theme::text_tertiary(),
        ));
    });
}

/// Where the runs ended, as profit on the capital: the median and the band's edges, and how often
/// a run lost.
fn key_figures(result: &MonteCarlo) -> [Figure; 4] {
    let ended = |equity: f64| {
        let profit = equity - result.initial_capital;
        (
            PineReportFormat::signed_money(profit),
            report::tint(profit, theme::text_primary()),
        )
    };
    let figure = |label, (value, color)| Figure::new(label, value, color);
    [
        figure("Median result", ended(result.final_equity.median)),
        figure("5th percentile", ended(result.final_equity.low)),
        figure("95th percentile", ended(result.final_equity.high)),
        Figure::new(
            "Chance of a loss",
            PineReportFormat::percent(result.loss_probability, false),
            theme::text_primary(),
        ),
    ]
}

/// How deep the runs fell from a peak: typically, and in the worst twentieth.
fn quiet_figures(result: &MonteCarlo) -> [Figure; 2] {
    let drawdown = |fraction| PineReportFormat::percent(fraction, false);
    [
        Figure::new(
            "Median drawdown",
            drawdown(result.max_drawdown.median),
            theme::text_secondary(),
        ),
        Figure::new(
            "Drawdown, worst 5%",
            drawdown(result.max_drawdown.high),
            theme::text_secondary(),
        ),
    ]
}

/// The middle 90% of the runs' equity along the trades as a band, the median through it, and the
/// initial capital as a dashed line; the pointer reads them after that many trades.
fn fan_chart(ui: &mut Ui, result: &MonteCarlo) {
    let low = result
        .fan
        .iter()
        .map(|step| step.low)
        .fold(f64::MAX, f64::min);
    let high = result
        .fan
        .iter()
        .map(|step| step.high)
        .fold(f64::MIN, f64::max);
    let (plot, hovered) = Plot::show(
        ui,
        FAN_HEIGHT,
        result.fan.len(),
        low,
        high,
        result.initial_capital,
    );
    let tone = plot::tone(result.final_equity.median, result.initial_capital);
    let band = theme::with_alpha(tone, BAND_ALPHA);
    for (step, pair) in result.fan.windows(2).enumerate() {
        let [from, to]: [&Spread; 2] = [&pair[0], &pair[1]];
        plot.painter().add(Shape::convex_polygon(
            vec![
                pos2(plot.x(step), plot.y(from.high)),
                pos2(plot.x(step + 1), plot.y(to.high)),
                pos2(plot.x(step + 1), plot.y(to.low)),
                pos2(plot.x(step), plot.y(from.low)),
            ],
            band,
            Stroke::NONE,
        ));
    }
    plot.line(result.fan.iter().map(|step| step.median), tone);
    if let Some(step) = hovered {
        let spread = result.fan[step];
        let span = high.max(result.initial_capital) - low.min(result.initial_capital);
        let axis = |value| PineReportFormat::axis(value, span);
        let trades = result.trades_at(step);
        plot.readout(
            step,
            &[(spread.median, tone)],
            &[
                format!("After {trades} trade{}", if trades == 1 { "" } else { "s" }),
                format!("Median {}", axis(spread.median)),
                format!("90% within {} to {}", axis(spread.low), axis(spread.high)),
            ],
        );
    }
}

/// What the band, the line and the dashes are, wrapping under each other when narrow.
fn legend(ui: &mut Ui, tone: Color32) {
    const SWATCH: Vec2 = vec2(14.0, 8.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(14.0, 4.0);
        let entries: [(&str, Key); 3] = [
            ("Middle 90% of runs", Key::Band),
            ("Median", Key::Line),
            ("Initial capital", Key::Dashes),
        ];
        for (text, key) in entries {
            let words = kit::galley(
                ui,
                text,
                kit::size::CAPTION,
                kit::REGULAR,
                theme::text_secondary(),
                f32::INFINITY,
            );
            let size = vec2(SWATCH.x + 5.0 + words.size().x, words.size().y);
            let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
            let swatch =
                Rect::from_center_size(pos2(rect.left() + SWATCH.x / 2.0, rect.center().y), SWATCH);
            let painter = ui.painter();
            let y = swatch.center().y;
            match key {
                Key::Band => {
                    painter.rect_filled(swatch, 2.0, theme::with_alpha(tone, BAND_ALPHA));
                }
                Key::Line => {
                    painter.hline(swatch.x_range(), y, Stroke::new(1.5, tone));
                }
                Key::Dashes => painter.extend(Shape::dashed_line(
                    &[pos2(swatch.left(), y), pos2(swatch.right(), y)],
                    Stroke::new(1.0, theme::with_alpha(theme::text_secondary(), 0.5)),
                    3.0,
                    3.0,
                )),
            }
            painter.galley(
                pos2(swatch.right() + 5.0, rect.top()),
                words,
                Color32::PLACEHOLDER,
            );
        }
    });
}

/// The mark a legend entry shows.
enum Key {
    Band,
    Line,
    Dashes,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pine::strategy::{StrategySettings, Trade};
    use eframe::egui;

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
    fn the_card_draws_with_too_few_trades_too_many_and_enough() {
        let few = report(&[5.0, -3.0]);
        let enough = report(&[5.0, -3.0, 8.0, -1.0, 4.0, -6.0, 9.0, -2.0, 7.0, -4.0, 3.0]);
        let too_many = report(&vec![1.0; MAX_TRADES + 1]);
        egui::__run_test_ui(|ui| {
            simulation_card(ui, &few);
            simulation_card(ui, &enough);
            simulation_card(ui, &too_many);
        });
    }

    #[test]
    fn a_refusal_is_cached_with_the_trades_it_was_for() {
        let too_many = report(&vec![1.0; MAX_TRADES + 1]);
        egui::__run_test_ui(|ui| {
            assert!(matches!(
                resampled(ui, &too_many),
                Err(Unavailable::TooMany { .. })
            ));
            assert!(matches!(
                resampled(ui, &too_many),
                Err(Unavailable::TooMany { .. })
            ));
            assert!(
                resampled(
                    ui,
                    &report(&[1.0, -1.0, 2.0, -2.0, 3.0, -3.0, 4.0, -4.0, 5.0, -5.0])
                )
                .is_ok()
            );
        });
    }
}
