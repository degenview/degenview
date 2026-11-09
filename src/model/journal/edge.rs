//! The Edge Score: LuxAlgo's published, versioned composite of trading performance (0 to 100),
//! version 2, from the open-source `LuxAlgo/trade-journal` repository (`packages/core/src/
//! edge-score.ts` and `docs/edge-score.md`; the repository states the MIT licence). Reimplemented
//! here natively from the published formula, not copied.
//!
//! Six components, each scaled to 0 to 100 against an explicit full-marks threshold, combined by
//! fixed weights that sum to 100. Changing a threshold, weight or input definition is a new
//! version. The score is withheld below [`MIN_TRADES`] closed trades (the components are still
//! shown) and the trade count always travels with it.
//!
//! | Component | Full marks | Weight |
//! | --- | --- | --- |
//! | Win rate | 60% | 15 |
//! | Profit factor | 3.0 (no losses: 3.0) | 25 |
//! | Average win / average loss | 2.5 : 1 | 20 |
//! | Drawdown | 0% of (initial balance + peak); zero at 25% or more | 15 |
//! | Recovery factor | net P&L = 3 × max drawdown | 10 |
//! | Consistency | largest winning day at most 15% of winning-day profit | 15 |
//!
//! Two cases where the published code and the published documentation disagree, and this
//! implementation follows the **documentation**, which states the intent:
//!
//! - *No drawdown at all* (with a known initial balance): the documentation gives full marks for
//!   0%; the reference code yields its neutral 50 because its percentage is left unset. Here: 100.
//! - *No initial balance*: the documentation says the percentage is unknowable and scores a
//!   neutral 50; the reference code measures against peak P&L alone. Here: 50.
//!
//! This is a measure of the record, not a forecast. It never uses subjective review ratings.

use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use super::analytics::Summary;
use super::metric::{Metric, Reason};

pub const EDGE_SCORE_VERSION: u32 = 2;
pub const MIN_TRADES: usize = 5;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ComponentKey {
    WinRate,
    ProfitFactor,
    AvgWinLoss,
    Drawdown,
    Recovery,
    Consistency,
}

pub const WEIGHTS: [(ComponentKey, u32); 6] = [
    (ComponentKey::WinRate, 15),
    (ComponentKey::ProfitFactor, 25),
    (ComponentKey::AvgWinLoss, 20),
    (ComponentKey::Drawdown, 15),
    (ComponentKey::Recovery, 10),
    (ComponentKey::Consistency, 15),
];

impl ComponentKey {
    pub fn label(self) -> &'static str {
        match self {
            Self::WinRate => "Win rate",
            Self::ProfitFactor => "Profit factor",
            Self::AvgWinLoss => "Avg win / avg loss",
            Self::Drawdown => "Drawdown",
            Self::Recovery => "Recovery",
            Self::Consistency => "Consistency",
        }
    }

    pub fn full_marks(self) -> &'static str {
        match self {
            Self::WinRate => "60% win rate",
            Self::ProfitFactor => "profit factor of 3.0",
            Self::AvgWinLoss => "winners 2.5 times the size of losers",
            Self::Drawdown => "no drawdown; zero at 25% or more",
            Self::Recovery => "net P&L of 3 times the max drawdown",
            Self::Consistency => "largest winning day at most 15% of winning-day profit",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Component {
    pub key: ComponentKey,
    /// 0 to 100.
    pub value: f64,
    pub weight: u32,
    /// The measured input the value came from, when there is one.
    pub input: Option<f64>,
    /// Why the value is not a plain function of the input (a neutral score, a missing input).
    pub note: Option<&'static str>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EdgeScore {
    pub version: u32,
    /// 0 to 100, two decimals; unavailable below [`MIN_TRADES`] closed trades.
    pub score: Metric<f64>,
    pub components: Vec<Component>,
    pub closed_trades: usize,
}

fn clamp01(value: f64) -> f64 {
    value.clamp(0.0, 1.0)
}

fn f(value: Decimal) -> f64 {
    value.to_f64().unwrap_or(0.0)
}

pub fn edge_score(summary: &Summary, initial_balance: Option<Decimal>) -> EdgeScore {
    let mut components = Vec::with_capacity(6);

    let win_rate = summary.win_rate.copied();
    components.push(Component {
        key: ComponentKey::WinRate,
        value: clamp01(win_rate.unwrap_or(0.0) / 0.6) * 100.0,
        weight: 15,
        input: win_rate,
        note: None,
    });

    let (pf_input, pf_note) = match summary.profit_factor.copied() {
        Some(pf) => (Some(pf), None),
        None if summary.gross_loss.is_zero() && summary.gross_profit > Decimal::ZERO => {
            (Some(3.0), Some("No losses: counted as 3.0."))
        }
        None => (None, None),
    };
    components.push(Component {
        key: ComponentKey::ProfitFactor,
        value: clamp01(pf_input.unwrap_or(0.0) / 3.0) * 100.0,
        weight: 25,
        input: pf_input,
        note: pf_note,
    });

    let payoff = summary.payoff.copied();
    components.push(Component {
        key: ComponentKey::AvgWinLoss,
        value: clamp01(payoff.unwrap_or(0.0) / 2.5) * 100.0,
        weight: 20,
        input: payoff,
        note: None,
    });

    let has_balance = initial_balance.is_some_and(|balance| balance > Decimal::ZERO);
    let (drawdown_value, drawdown_input, drawdown_note) = if !has_balance {
        (50.0, None, Some("No initial balance: neutral 50."))
    } else if summary.drawdown.max.is_zero() {
        (100.0, Some(0.0), None)
    } else {
        match summary.drawdown.max_pct {
            Some(pct) => ((1.0 - clamp01(pct / 0.25)) * 100.0, Some(pct), None),
            None => (50.0, None, Some("Percentage unknowable: neutral 50.")),
        }
    };
    components.push(Component {
        key: ComponentKey::Drawdown,
        value: drawdown_value,
        weight: 15,
        input: drawdown_input,
        note: drawdown_note,
    });

    let (recovery_value, recovery_input) = if summary.drawdown.max > Decimal::ZERO {
        let factor = f(summary.net_pnl) / f(summary.drawdown.max);
        (clamp01(factor / 3.0) * 100.0, Some(factor))
    } else if summary.net_pnl > Decimal::ZERO {
        (100.0, None)
    } else {
        (0.0, None)
    };
    components.push(Component {
        key: ComponentKey::Recovery,
        value: recovery_value,
        weight: 10,
        input: recovery_input,
        note: summary
            .drawdown
            .max
            .is_zero()
            .then_some("No drawdown: full marks only if net P&L is positive."),
    });

    let concentration = summary.days.profit_concentration.copied();
    let consistency = match concentration {
        None => 0.0,
        Some(c) if c <= 0.15 => 100.0,
        Some(c) => (1.0 - clamp01((c - 0.15) / 0.85)) * 100.0,
    };
    components.push(Component {
        key: ComponentKey::Consistency,
        value: consistency,
        weight: 15,
        input: concentration,
        note: concentration.is_none().then_some("No winning day."),
    });

    let total_weight: u32 = WEIGHTS.iter().map(|(_, weight)| weight).sum();
    let weighted: f64 = components
        .iter()
        .map(|component| component.value * f64::from(component.weight))
        .sum::<f64>()
        / f64::from(total_weight);
    let score = if summary.closed >= MIN_TRADES {
        Metric::Value((weighted * 100.0).round() / 100.0)
    } else {
        Metric::Unavailable(Reason::Insufficient {
            have: summary.closed,
            need: MIN_TRADES,
        })
    };
    EdgeScore {
        version: EDGE_SCORE_VERSION,
        score,
        components,
        closed_trades: summary.closed,
    }
}

#[cfg(test)]
mod tests;
