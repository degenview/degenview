//! The Process score: how closely the trader followed their own process, from what they recorded in
//! reviews. Pure, and deliberately **separate from the Edge Score**, which reads only results. The
//! two never blend, and a subjective rating never changes an objective figure.
//!
//! Three components over the *reviewed* closed trades (a trade is reviewed when its review has
//! anything written or ticked):
//!
//! | Component | Input | Weight |
//! | --- | --- | --- |
//! | Rule adherence | mean checklist share over trades with a playbook checklist | 40 |
//! | Discipline | mean discipline rating (1 to 5) scaled to 0 to 100 | 30 |
//! | Clean execution | share of trades with no mistake noted | 30 |
//!
//! A component with no inputs is left out and the weights of the rest are renormalised; the score is
//! withheld below [`MIN_REVIEWED`] reviewed trades. It is a record of self-reported process, not a
//! measure of skill.

use std::collections::HashMap;

use super::metric::{Metric, Reason};
use super::model::{Trade, TradeStatus};
use super::playbook::{Playbook, rule_adherence};
use super::reviews::Review;

pub const MIN_REVIEWED: usize = 10;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessKey {
    RuleAdherence,
    Discipline,
    CleanExecution,
}

impl ProcessKey {
    pub fn label(self) -> &'static str {
        match self {
            Self::RuleAdherence => "Rule adherence",
            Self::Discipline => "Discipline",
            Self::CleanExecution => "Clean execution",
        }
    }

    pub fn definition(self) -> &'static str {
        match self {
            Self::RuleAdherence => {
                "Mean share of the playbook checklist ticked, over trades with a playbook that has a checklist."
            }
            Self::Discipline => "Mean of your 1 to 5 discipline ratings, scaled to 0 to 100.",
            Self::CleanExecution => "Share of reviewed trades with no mistake noted.",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProcessComponent {
    pub key: ProcessKey,
    pub weight: u32,
    /// 0 to 100, or `None` when nothing was recorded for it.
    pub value: Option<f64>,
    /// How many trades the value is over.
    pub samples: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProcessScore {
    pub score: Metric<f64>,
    pub components: Vec<ProcessComponent>,
    pub reviewed: usize,
    pub closed: usize,
}

pub fn process_score(
    trades: &[&Trade],
    reviews: &HashMap<String, Review>,
    playbooks: &[Playbook],
) -> ProcessScore {
    let closed: Vec<&Trade> = trades
        .iter()
        .copied()
        .filter(|trade| trade.status == TradeStatus::Closed)
        .collect();
    let reviewed: Vec<(&Trade, &Review)> = closed
        .iter()
        .filter_map(|trade| {
            reviews
                .get(&trade.key)
                .filter(|review| !review.is_empty())
                .map(|review| (*trade, review))
        })
        .collect();

    let adherence: Vec<f64> = reviewed
        .iter()
        .filter_map(|(_, review)| {
            let playbook = playbooks
                .iter()
                .find(|p| Some(p.id.as_str()) == review.playbook.as_deref())?;
            rule_adherence(review, playbook)
        })
        .collect();
    let discipline: Vec<f64> = reviewed
        .iter()
        .filter_map(|(_, review)| review.discipline)
        .map(|rating| f64::from(rating - 1) / 4.0)
        .collect();
    let clean = reviewed
        .iter()
        .filter(|(_, review)| review.mistakes.is_empty())
        .count();

    let mean = |values: &[f64]| {
        (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64 * 100.0)
    };
    let components = vec![
        ProcessComponent {
            key: ProcessKey::RuleAdherence,
            weight: 40,
            value: mean(&adherence),
            samples: adherence.len(),
        },
        ProcessComponent {
            key: ProcessKey::Discipline,
            weight: 30,
            value: mean(&discipline),
            samples: discipline.len(),
        },
        ProcessComponent {
            key: ProcessKey::CleanExecution,
            weight: 30,
            value: (!reviewed.is_empty()).then(|| clean as f64 / reviewed.len() as f64 * 100.0),
            samples: reviewed.len(),
        },
    ];
    let weight: u32 = components
        .iter()
        .filter(|c| c.value.is_some())
        .map(|c| c.weight)
        .sum();
    let score = if reviewed.len() < MIN_REVIEWED {
        Metric::Unavailable(Reason::Insufficient {
            have: reviewed.len(),
            need: MIN_REVIEWED,
        })
    } else if weight == 0 {
        Metric::Unavailable(Reason::NotApplicable)
    } else {
        let total: f64 = components
            .iter()
            .filter_map(|c| c.value.map(|v| v * f64::from(c.weight)))
            .sum();
        Metric::Value((total / f64::from(weight) * 100.0).round() / 100.0)
    };
    ProcessScore {
        score,
        components,
        reviewed: reviewed.len(),
        closed: closed.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::journal::model::{CostBasis, Side};
    use crate::model::journal::reconstruct::reconstruct;
    use crate::model::journal::testing::exec;

    fn trades(n: usize) -> Vec<Trade> {
        let mut e = Vec::new();
        for i in 0..n {
            let t = (i as i64 + 1) * 10_000;
            e.push(exec(&format!("{i}a"), Side::Buy, "1", "100", t));
            e.push(exec(&format!("{i}b"), Side::Sell, "1", "101", t + 5_000));
        }
        reconstruct(&e, |_| CostBasis::AverageCost)
    }

    fn playbook() -> Playbook {
        let mut p = Playbook::new("p".into(), "Breakout");
        p.checklist = vec!["a".into(), "b".into()];
        p
    }

    #[test]
    fn it_is_withheld_until_enough_trades_are_reviewed() {
        let trades = trades(12);
        let refs: Vec<&Trade> = trades.iter().collect();
        let mut reviews = HashMap::new();
        for trade in trades.iter().take(9) {
            let mut r = Review::new(&trade.key, "acct");
            r.notes = "x".into();
            reviews.insert(trade.key.clone(), r);
        }
        let score = process_score(&refs, &reviews, &[]);
        assert_eq!(score.reviewed, 9);
        assert_eq!(score.closed, 12);
        assert_eq!(
            score.score,
            Metric::Unavailable(Reason::Insufficient { have: 9, need: 10 })
        );
        assert!(
            process_score(&refs, &HashMap::new(), &[])
                .score
                .reason()
                .is_some()
        );
    }

    #[test]
    fn components_are_averages_over_what_was_recorded() {
        let trades = trades(10);
        let refs: Vec<&Trade> = trades.iter().collect();
        let pb = playbook();
        let mut reviews = HashMap::new();
        for (i, trade) in trades.iter().enumerate() {
            let mut r = Review::new(&trade.key, "acct");
            r.playbook = Some("p".into());
            // Half the trades tick both rules, half tick one.
            r.checks = if i % 2 == 0 {
                ["a".to_owned(), "b".to_owned()].into()
            } else {
                ["a".to_owned()].into()
            };
            r.discipline = Some(if i % 2 == 0 { 5 } else { 3 });
            if i < 2 {
                r.mistakes = vec!["fomo".into()];
            }
            reviews.insert(trade.key.clone(), r);
        }
        let score = process_score(&refs, &reviews, &[pb]);
        let value = |key: ProcessKey| {
            score
                .components
                .iter()
                .find(|c| c.key == key)
                .unwrap()
                .value
                .unwrap()
        };
        assert!((value(ProcessKey::RuleAdherence) - 75.0).abs() < 1e-9);
        // (5 -> 100, 3 -> 50): mean 75.
        assert!((value(ProcessKey::Discipline) - 75.0).abs() < 1e-9);
        assert!((value(ProcessKey::CleanExecution) - 80.0).abs() < 1e-9);
        let expected = (75.0 * 40.0 + 75.0 * 30.0 + 80.0 * 30.0) / 100.0;
        assert!((score.score.copied().unwrap() - expected).abs() < 1e-9);
    }

    #[test]
    fn a_missing_component_is_left_out_and_the_rest_renormalised() {
        let trades = trades(10);
        let refs: Vec<&Trade> = trades.iter().collect();
        let mut reviews = HashMap::new();
        for trade in &trades {
            let mut r = Review::new(&trade.key, "acct");
            r.notes = "x".into();
            reviews.insert(trade.key.clone(), r);
        }
        let score = process_score(&refs, &reviews, &[]);
        assert!(score.components[0].value.is_none() && score.components[1].value.is_none());
        // Only clean execution has inputs, and every trade is clean.
        assert_eq!(score.score, Metric::Value(100.0));
    }

    #[test]
    fn open_trades_and_empty_reviews_do_not_count() {
        let mut t = trades(10);
        t.extend(reconstruct(
            &[exec("o", Side::Buy, "1", "100", 999_999)],
            |_| CostBasis::AverageCost,
        ));
        let refs: Vec<&Trade> = t.iter().collect();
        let mut reviews = HashMap::new();
        for trade in &t {
            reviews.insert(trade.key.clone(), Review::new(&trade.key, "acct"));
        }
        let score = process_score(&refs, &reviews, &[]);
        assert_eq!(score.closed, 10);
        assert_eq!(score.reviewed, 0);
    }
}
