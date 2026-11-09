//! Two more exports: the figures as a CSV, and the user's own writing as Markdown.
//!
//! The summary has one row per figure per currency, with the reason in a `note` column where a
//! figure does not exist (never a zero or a blank that looks like one). Amounts are never summed
//! across currencies: each currency has its own rows.

use std::collections::HashMap;

use rust_decimal::Decimal;

use super::analytics::Summary;
use super::calendar::{Offset, civil_from_days, day_number};
use super::daily::DailyEntry;
use super::filters::{OutcomeFilter, outcome_of};
use super::format;
use super::metric::Metric;
use super::model::{Direction, Trade};
use super::reviews::Review;
use crate::model::portfolio_csv::escape;

pub const SUMMARY_HEADER: &str = "metric,currency,value,note";

fn line(out: &mut String, metric: &str, currency: &str, value: String, note: &str) {
    let fields = [
        metric.to_owned(),
        currency.to_owned(),
        value,
        note.to_owned(),
    ];
    out.push_str(
        &fields
            .iter()
            .map(|field| escape(field))
            .collect::<Vec<_>>()
            .join(","),
    );
    out.push('\n');
}

/// One row per figure for each currency's summary.
pub fn summary_csv(summaries: &[(Option<String>, Summary)]) -> String {
    let mut out = String::from(SUMMARY_HEADER);
    out.push('\n');
    for (currency, summary) in summaries {
        let currency = currency.clone().unwrap_or_default();
        let c = currency.as_str();
        let count = |out: &mut String, name: &str, value: usize| {
            line(out, name, c, value.to_string(), "");
        };
        let money = |out: &mut String, name: &str, value: Decimal, note: &str| {
            line(out, name, c, value.normalize().to_string(), note);
        };
        let metric_f = |out: &mut String, name: &str, value: &Metric<f64>| match value {
            Metric::Value(number) => line(out, name, c, format!("{number:.6}"), ""),
            Metric::Unavailable(reason) => line(out, name, c, String::new(), &reason.explain()),
        };
        let metric_d = |out: &mut String, name: &str, value: &Metric<Decimal>| match value {
            Metric::Value(amount) => line(out, name, c, amount.normalize().to_string(), ""),
            Metric::Unavailable(reason) => line(out, name, c, String::new(), &reason.explain()),
        };
        count(&mut out, "closed_trades", summary.closed);
        count(&mut out, "open_trades", summary.open);
        count(&mut out, "wins", summary.wins);
        count(&mut out, "losses", summary.losses);
        count(&mut out, "breakevens", summary.breakevens);
        money(&mut out, "gross_pnl", summary.gross_pnl, "");
        money(
            &mut out,
            "fees",
            summary.fees,
            if summary.fees_complete {
                ""
            } else {
                "some fills had no fee figure: fees and net are a floor"
            },
        );
        money(&mut out, "net_pnl", summary.net_pnl, "");
        money(&mut out, "gross_profit", summary.gross_profit, "");
        money(&mut out, "gross_loss", summary.gross_loss, "");
        metric_f(&mut out, "win_rate", &summary.win_rate);
        metric_f(&mut out, "profit_factor", &summary.profit_factor);
        metric_d(&mut out, "average_win", &summary.average_win);
        metric_d(&mut out, "average_loss", &summary.average_loss);
        metric_f(&mut out, "payoff_ratio", &summary.payoff);
        metric_d(&mut out, "expectancy", &summary.expectancy);
        count(&mut out, "trades_with_r", summary.r.count);
        metric_f(&mut out, "average_r", &summary.r.average);
        metric_f(&mut out, "best_r", &summary.r.best);
        metric_f(&mut out, "worst_r", &summary.r.worst);
        metric_f(&mut out, "total_r", &summary.r.total);
        metric_f(&mut out, "expectancy_r", &summary.expectancy_r);
        money(&mut out, "max_drawdown", summary.drawdown.max, "");
        match summary.drawdown.max_pct {
            Some(pct) => line(&mut out, "max_drawdown_pct", c, format!("{pct:.6}"), ""),
            None => line(
                &mut out,
                "max_drawdown_pct",
                c,
                String::new(),
                "needs an initial balance and a drawdown",
            ),
        }
        count(&mut out, "max_win_streak", summary.streaks.max_wins);
        count(&mut out, "max_loss_streak", summary.streaks.max_losses);
        count(&mut out, "trading_days", summary.days.trading_days);
        metric_f(&mut out, "day_win_rate", &summary.days.day_win_rate);
        match &summary.durations.average_ms {
            Metric::Value(ms) => line(
                &mut out,
                "average_hold_seconds",
                c,
                (ms / 1_000).to_string(),
                "",
            ),
            Metric::Unavailable(reason) => {
                line(
                    &mut out,
                    "average_hold_seconds",
                    c,
                    String::new(),
                    &reason.explain(),
                );
            }
        }
        metric_f(&mut out, "sharpe", &summary.sharpe);
        metric_f(&mut out, "sortino", &summary.sortino);
        metric_f(&mut out, "calmar", &summary.calmar);
        metric_f(&mut out, "sqn", &summary.sqn);
    }
    out
}

fn iso_date(ts_ms: i64, offset: Offset<'_>) -> String {
    let civil = civil_from_days(day_number(ts_ms, offset));
    format!("{:04}-{:02}-{:02}", civil.year, civil.month, civil.day)
}

/// What the user wrote, as Markdown: each day's entry and each reviewed trade's notes, oldest
/// first. Trades with nothing written are left out, and so is any section with nothing in it.
pub fn notes_markdown(
    trades: &[&Trade],
    reviews: &HashMap<String, Review>,
    days: &[&DailyEntry],
    playbook_name: &dyn Fn(&str) -> String,
    account_name: &dyn Fn(&str) -> String,
    offset: Offset<'_>,
    exported_on: &str,
) -> String {
    let mut out = format!("# Trade Journal notes\n\nExported {exported_on}.\n");
    let mut days: Vec<&&DailyEntry> = days.iter().collect();
    days.sort_by(|a, b| a.date.cmp(&b.date));
    if !days.is_empty() {
        out.push_str("\n## Daily journal\n");
        for day in days {
            out.push_str(&format!("\n### {}\n", day.date));
            if !day.mood.is_empty() {
                out.push_str(&format!("\nMood: {}\n", day.mood.join(", ")));
            }
            for (heading, text) in [
                ("Outlook", &day.outlook),
                ("Plan", &day.plan),
                ("Goals", &day.goals),
                ("Session notes", &day.notes),
                ("Mistakes", &day.mistakes),
                ("Lessons", &day.lessons),
                ("End-of-day review", &day.review),
            ] {
                if !text.trim().is_empty() {
                    out.push_str(&format!("\n**{heading}**\n\n{}\n", text.trim_end()));
                }
            }
        }
    }
    let mut written: Vec<(&&Trade, &Review)> = trades
        .iter()
        .filter_map(|trade| {
            reviews
                .get(&trade.key)
                .filter(|review| has_words(review))
                .map(|review| (trade, review))
        })
        .collect();
    written.sort_by_key(|(trade, _)| (trade.open_ts_ms, trade.key.clone()));
    if !written.is_empty() {
        out.push_str("\n## Trades\n");
        for (trade, review) in written {
            let direction = match trade.direction {
                Direction::Long => "Long",
                Direction::Short => "Short",
            };
            let result = if outcome_of(trade, Decimal::ZERO) == OutcomeFilter::Open {
                "open".to_owned()
            } else {
                format!(
                    "{} {}",
                    format::signed_money(trade.net_pnl()),
                    trade.currency.clone().unwrap_or_default()
                )
                .trim()
                .to_owned()
            };
            out.push_str(&format!(
                "\n### {} \u{b7} {} \u{b7} {direction} \u{b7} {result}\n\n{}\n",
                iso_date(trade.open_ts_ms, offset),
                trade.instrument.display_name,
                account_name(&trade.account_uid)
            ));
            let mut facts = Vec::new();
            if let Some(playbook) = review.playbook.as_deref() {
                facts.push(format!("Playbook: {}", playbook_name(playbook)));
            }
            if let Some(rating) = review.rating {
                facts.push(format!("Rating {rating}/5"));
            }
            if let Some(discipline) = review.discipline {
                facts.push(format!("Discipline {discipline}/5"));
            }
            if review.reviewed {
                facts.push("Reviewed".to_owned());
            }
            if !facts.is_empty() {
                out.push_str(&format!("\n{}\n", facts.join(" \u{b7} ")));
            }
            for (label, items) in [
                ("Tags", &review.tags),
                ("Mistakes", &review.mistakes),
                ("Emotions", &review.emotions),
            ] {
                if !items.is_empty() {
                    out.push_str(&format!("\n{label}: {}\n", items.join(", ")));
                }
            }
            for (heading, text) in [
                ("Setup", &review.setup),
                ("Why I entered", &review.entry_rationale),
                ("Why I exited", &review.exit_rationale),
                ("Notes", &review.notes),
            ] {
                if !text.trim().is_empty() {
                    out.push_str(&format!("\n**{heading}**\n\n{}\n", text.trim_end()));
                }
            }
        }
    }
    out
}

fn has_words(review: &Review) -> bool {
    [
        &review.setup,
        &review.entry_rationale,
        &review.exit_rationale,
        &review.notes,
    ]
    .iter()
    .any(|text| !text.trim().is_empty())
        || !review.tags.is_empty()
        || !review.mistakes.is_empty()
        || !review.emotions.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::journal::analytics::{Options, summarize_by_currency};
    use crate::model::journal::csv::{ColumnMap, ImportOptions, interpret, plan, read_table};
    use crate::model::journal::model::CostBasis;
    use crate::model::journal::reconstruct::reconstruct;
    use crate::model::portfolio_csv::parse_records;

    fn trades() -> Vec<Trade> {
        let table = read_table(
            "time,symbol,side,qty,price\n\
             2026-11-09T10:00:00Z,BTC,buy,1,100\n\
             2026-11-09T11:00:00Z,BTC,sell,1,110\n\
             2026-11-10T10:00:00Z,ETH,buy,1,50\n",
        )
        .unwrap();
        let options = ImportOptions::default();
        let outcomes = interpret(&table, &ColumnMap::detect(&table.headers), &options, &|_| 0);
        reconstruct(&plan(&outcomes, &options, 1).executions, |_: &str| {
            CostBasis::AverageCost
        })
    }

    #[test]
    fn the_summary_has_a_row_per_figure_and_says_why_a_figure_is_missing() {
        let trades = trades();
        let refs: Vec<&Trade> = trades.iter().collect();
        let summaries = summarize_by_currency(&refs, &Options::default());
        let text = summary_csv(&summaries);
        let records = parse_records(&text);
        assert_eq!(records[0].join(","), SUMMARY_HEADER);
        assert!(records.iter().all(|record| record.len() == 4));
        let value = |name: &str| {
            records
                .iter()
                .find(|record| record[0] == name)
                .unwrap_or_else(|| panic!("{name}"))
                .clone()
        };
        assert_eq!(value("closed_trades")[2], "1");
        assert_eq!(value("open_trades")[2], "1");
        assert_eq!(value("net_pnl")[2], "10");
        // One win and no losses: the profit factor does not exist, and the row says so.
        let pf = value("profit_factor");
        assert_eq!(pf[2], "");
        assert!(pf[3].contains("losing"), "{pf:?}");
        // No initial balance: no drawdown percentage.
        assert!(value("max_drawdown_pct")[3].contains("initial balance"));
        assert_eq!(value("net_pnl")[1], "USD");
    }

    #[test]
    fn the_notes_hold_what_was_written_and_nothing_else() {
        let trades = trades();
        let refs: Vec<&Trade> = trades.iter().collect();
        let mut review = Review::new(&trades[0].key, &trades[0].account_uid);
        review.notes = "Waited for the retest.".into();
        review.tags = vec!["retest".into()];
        review.rating = Some(5);
        review.playbook = Some("pb".into());
        // A review with only a rating has no words and is left out.
        let mut quiet = Review::new(&trades[1].key, &trades[1].account_uid);
        quiet.rating = Some(2);
        let reviews = HashMap::from([
            (trades[0].key.clone(), review),
            (trades[1].key.clone(), quiet),
        ]);
        let mut day = DailyEntry::new("2026-11-09");
        day.plan = "Open only.".into();
        day.mood = vec!["Calm".into()];
        let text = notes_markdown(
            &refs,
            &reviews,
            &[&day],
            &|_| "Breakout".to_owned(),
            &|_| "Main".to_owned(),
            &|_| 0,
            "2026-11-10",
        );
        assert!(text.contains("### 2026-11-09\n"));
        assert!(text.contains("Mood: Calm"));
        assert!(text.contains("**Plan**\n\nOpen only."));
        assert!(text.contains("### 2026-11-09 \u{b7} BTC \u{b7} Long \u{b7} +10.00 USD"));
        assert!(text.contains("Playbook: Breakout \u{b7} Rating 5/5"));
        assert!(text.contains("Tags: retest"));
        assert!(text.contains("Waited for the retest."));
        assert_eq!(text.matches("### ").count(), 2, "{text}");
        let empty = notes_markdown(
            &[],
            &HashMap::new(),
            &[],
            &|_| String::new(),
            &|_| String::new(),
            &|_| 0,
            "x",
        );
        assert!(!empty.contains("## "));
    }
}
