//! Monte Carlo on a finished backtest: the closed trades are redrawn at random, with replacement,
//! many times over, to show how far the result could move had the wins and losses come out
//! differently.
//!
//! Each run draws as many trades as the strategy closed and adds their profits to the initial
//! capital one after another. Drawing with replacement changes how often each trade turns up, which
//! is what spreads the final equity; the order of the draws only shapes the path, so it matters to
//! the drawdown and not to where a run ends. Profits are taken as money, so a strategy that sizes by
//! percent of equity is only approximated: a redrawn run does not compound. The figures describe the
//! sample of trades, not the market; they say nothing about trades the strategy has not made.
//!
//! The random stream is fixed, so one report always gives the same figures. The work is bounded:
//! never more than [`DRAW_BUDGET`] draws, and a list too long for that many runs is refused.

use super::StrategyReport;

/// The fewest closed trades worth resampling; below this the spread is noise.
pub const MIN_TRADES: usize = 10;
/// The most runs, used while the trade list is short.
const MAX_RUNS: usize = 1_000;
/// The fewest runs worth a percentile.
const MIN_RUNS: usize = 100;
/// A budget of drawn trades across all runs, so a long list takes fewer runs instead of longer.
const DRAW_BUDGET: usize = 2_000_000;
/// The most closed trades resampled: more would leave fewer than [`MIN_RUNS`] runs in the budget.
pub const MAX_TRADES: usize = DRAW_BUDGET / MIN_RUNS;
/// How many steps the fan of equity paths is sampled at.
const FAN_STEPS: usize = 40;
const SEED: u64 = 0x9E37_79B9_7F4A_7C15;

/// The 5th, 50th and 95th percentile of a quantity across the runs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spread {
    pub low: f64,
    pub median: f64,
    pub high: f64,
}

impl Spread {
    /// Sorts `values` and reads the percentiles by nearest rank.
    fn of(values: &mut [f64]) -> Self {
        values.sort_unstable_by(f64::total_cmp);
        let at = |fraction: f64| values[((values.len() - 1) as f64 * fraction).round() as usize];
        Self {
            low: at(0.05),
            median: at(0.5),
            high: at(0.95),
        }
    }
}

/// Why a report is not resampled.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum Unavailable {
    #[error("Needs {} closed trades, {have} so far", MIN_TRADES)]
    TooFew { have: usize },
    #[error("Too many closed trades to reshuffle here")]
    TooMany { have: usize },
}

#[derive(Clone, Debug, PartialEq)]
pub struct MonteCarlo {
    pub runs: usize,
    pub trades: usize,
    pub initial_capital: f64,
    /// Equity after the last trade.
    pub final_equity: Spread,
    /// The largest fall from a peak, as a fraction of the peak. The peak starts at the initial
    /// capital, as in the report's own drawdown, and the fall is not capped: equity that goes
    /// below zero gives more than 1.
    pub max_drawdown: Spread,
    /// The share of runs that ended below the initial capital, rounding error aside: a run that
    /// breaks even is not a loss.
    pub loss_probability: f64,
    /// Equity at evenly spaced points along the trades, from the initial capital to the end.
    pub fan: Vec<Spread>,
}

impl MonteCarlo {
    /// Refused while the strategy has closed fewer than [`MIN_TRADES`] or more than [`MAX_TRADES`].
    pub fn run(report: &StrategyReport) -> Result<Self, Unavailable> {
        let profits: Vec<f64> = report.trades.iter().map(|trade| trade.profit).collect();
        let count = profits.len();
        if count < MIN_TRADES {
            return Err(Unavailable::TooFew { have: count });
        }
        if count > MAX_TRADES {
            return Err(Unavailable::TooMany { have: count });
        }
        let initial = report.settings.initial_capital;
        let runs = (DRAW_BUDGET / count).clamp(MIN_RUNS, MAX_RUNS);
        let steps = FAN_STEPS.min(count);
        let marks: Vec<usize> = (0..=steps).map(|step| step * count / steps).collect();
        // The most `count` additions of profits this large can be off by.
        let largest = profits
            .iter()
            .fold(0.0_f64, |most, profit| most.max(profit.abs()));
        let rounding = f64::EPSILON * (count * count) as f64 * largest;

        let mut stream = Stream(SEED);
        let mut at_mark = vec![Vec::with_capacity(runs); marks.len()];
        let mut drawdowns = Vec::with_capacity(runs);
        let mut losses = 0;
        for _ in 0..runs {
            let (mut gained, mut peak, mut worst) = (0.0_f64, initial, 0.0_f64);
            at_mark[0].push(initial);
            let mut next_mark = 1;
            for drawn in 1..=count {
                gained += profits[stream.below(count)];
                let equity = initial + gained;
                peak = peak.max(equity);
                if peak > 0.0 {
                    worst = worst.max((peak - equity) / peak);
                }
                if drawn == marks[next_mark] {
                    at_mark[next_mark].push(equity);
                    next_mark += 1;
                }
            }
            drawdowns.push(worst);
            // From the gain itself, not from `equity < initial`: two large numbers that should be
            // equal differ by rounding, and a run that broke even would read as a loss.
            if gained < -rounding {
                losses += 1;
            }
        }

        let fan: Vec<Spread> = at_mark.iter_mut().map(|row| Spread::of(row)).collect();
        Ok(Self {
            runs,
            trades: count,
            initial_capital: initial,
            final_equity: fan[steps],
            max_drawdown: Spread::of(&mut drawdowns),
            loss_probability: losses as f64 / runs as f64,
            fan,
        })
    }

    /// How many trades in the fan's point `step` stands, the same marks the runs were read at.
    pub fn trades_at(&self, step: usize) -> usize {
        let steps = self.fan.len().saturating_sub(1).max(1);
        step.min(steps) * self.trades / steps
    }
}

/// SplitMix64: a small, well-mixed generator. It needs no seed from the system, so the figures
/// repeat.
struct Stream(u64);

impl Stream {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A number in `0..bound` (`bound` below 2^32).
    fn below(&mut self, bound: usize) -> usize {
        (((self.next() >> 32) * bound as u64) >> 32) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pine::strategy::{StrategySettings, Trade};

    fn report(profits: &[f64]) -> StrategyReport {
        StrategyReport {
            settings: StrategySettings {
                initial_capital: 1_000.0,
                ..Default::default()
            },
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
    fn too_few_trades_are_not_resampled() {
        assert_eq!(
            MonteCarlo::run(&report(&[10.0; MIN_TRADES - 1])),
            Err(Unavailable::TooFew {
                have: MIN_TRADES - 1
            })
        );
        assert!(MonteCarlo::run(&report(&[10.0; MIN_TRADES])).is_ok());
    }

    #[test]
    fn a_run_that_breaks_even_is_not_a_loss() {
        // One loser among nine winners nets exactly zero, which 0.01 and 0.09 only approximate.
        // The same list in whole cents adds exactly, and draws the same trades.
        let fractional = report(&[0.01, 0.01, 0.01, 0.01, 0.01, 0.01, 0.01, 0.01, 0.01, -0.09]);
        let mut cents = report(&[1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, -9.0]);
        cents.settings.initial_capital = 100_000.0;
        let fractional = MonteCarlo::run(&fractional).unwrap();
        let cents = MonteCarlo::run(&cents).unwrap();
        assert_eq!(fractional.loss_probability, cents.loss_probability);
        assert!(fractional.loss_probability < 0.3);
    }

    #[test]
    fn equity_below_zero_gives_a_drawdown_above_a_whole() {
        let result = MonteCarlo::run(&report(&[-150.0; 10])).unwrap();
        assert_eq!(result.final_equity.median, -500.0);
        assert_eq!(result.max_drawdown.high, 1.5);
    }

    #[test]
    fn a_list_the_budget_cannot_cover_is_refused() {
        let longest = MonteCarlo::run(&report(&vec![5.0; MAX_TRADES])).unwrap();
        assert_eq!(longest.runs, MIN_RUNS);
        assert!(longest.runs * longest.trades <= DRAW_BUDGET);
        assert_eq!(
            MonteCarlo::run(&report(&vec![5.0; MAX_TRADES + 1])),
            Err(Unavailable::TooMany {
                have: MAX_TRADES + 1
            })
        );
    }

    #[test]
    fn equal_trades_leave_no_spread() {
        let result = MonteCarlo::run(&report(&[10.0; 20])).unwrap();
        let end = Spread {
            low: 1_200.0,
            median: 1_200.0,
            high: 1_200.0,
        };
        assert_eq!(result.final_equity, end);
        assert_eq!(result.max_drawdown.high, 0.0);
        assert_eq!(result.loss_probability, 0.0);
    }

    #[test]
    fn losing_trades_always_lose() {
        let result = MonteCarlo::run(&report(&[-10.0; 20])).unwrap();
        assert_eq!(result.loss_probability, 1.0);
        assert_eq!(result.final_equity.median, 800.0);
        assert!((result.max_drawdown.median - 0.2).abs() < 1e-9);
    }

    #[test]
    fn a_mixed_list_spreads_and_keeps_its_mean() {
        let profits: Vec<f64> = (0..50)
            .map(|index| if index % 2 == 0 { 30.0 } else { -20.0 })
            .collect();
        let result = MonteCarlo::run(&report(&profits)).unwrap();
        let mean = 1_000.0 + profits.iter().sum::<f64>();
        let spread = result.final_equity;
        assert!(spread.low < mean && mean < spread.high);
        assert!((spread.median - mean).abs() < 100.0);
        assert!(result.max_drawdown.low <= result.max_drawdown.median);
        assert!(result.max_drawdown.median <= result.max_drawdown.high);
    }

    #[test]
    fn the_fan_runs_from_the_capital_to_the_final_equity() {
        let result = MonteCarlo::run(&report(&[
            15.0, -5.0, 8.0, -12.0, 20.0, -3.0, 9.0, -7.0, 11.0, -2.0, 6.0, -9.0,
        ]))
        .unwrap();
        assert_eq!(result.fan.len(), result.trades.min(FAN_STEPS) + 1);
        assert_eq!(result.fan[0].median, 1_000.0);
        assert_eq!(result.fan.last(), Some(&result.final_equity));
        assert_eq!(result.trades_at(0), 0);
        assert_eq!(result.trades_at(result.fan.len() - 1), result.trades);
        for step in &result.fan {
            assert!(step.low <= step.median && step.median <= step.high);
        }
    }

    #[test]
    fn the_same_report_gives_the_same_figures() {
        let list = report(&[
            12.0, -7.0, 3.0, 25.0, -14.0, 9.0, -2.0, 18.0, -6.0, 4.0, 1.0,
        ]);
        assert_eq!(MonteCarlo::run(&list), MonteCarlo::run(&list));
    }

    #[test]
    fn a_long_list_takes_fewer_runs() {
        let few = MonteCarlo::run(&report(&[5.0; 20])).unwrap();
        let many = MonteCarlo::run(&report(&vec![5.0; 5_000])).unwrap();
        assert_eq!(few.runs, MAX_RUNS);
        assert_eq!(many.runs, DRAW_BUDGET / 5_000);
    }

    #[test]
    fn draws_cover_the_whole_list() {
        let mut stream = Stream(SEED);
        let mut seen = [false; 7];
        for _ in 0..200 {
            seen[stream.below(7)] = true;
        }
        assert!(seen.iter().all(|&hit| hit));
    }
}
