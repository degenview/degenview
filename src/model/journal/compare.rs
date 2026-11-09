//! Accounts side by side. Pure. Each column is one account's summary, Edge Score and equity path
//! over the same selection, in one currency; accounts in another currency are left out rather than
//! converted.
//!
//! Equity paths start at zero, so two accounts of different sizes compare on what they earned and
//! how it moved, not on how large they are.

use std::collections::HashMap;

use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use super::analytics::{Options, Summary, currency_of, summarize};
use super::calendar::Offset;
use super::edge::{EdgeScore, edge_score};
use super::equity::equity_curve;
use super::model::{JournalAccount, Trade};
use super::reviews::Review;
use super::risk;

#[derive(Clone, Debug)]
pub struct AccountColumn {
    pub uid: String,
    pub name: String,
    pub archived: bool,
    pub initial_balance: Option<Decimal>,
    pub summary: Summary,
    pub edge: EdgeScore,
    /// `(close time, cumulative net P&L)`.
    pub equity: Vec<(i64, f64)>,
}

pub struct Input<'a> {
    pub accounts: &'a [JournalAccount],
    /// The trades after the range and filters, any account.
    pub trades: &'a [&'a Trade],
    pub currency: Option<String>,
    pub reviews: &'a HashMap<String, Review>,
    pub breakeven_tolerance: Decimal,
    pub offset: Offset<'a>,
}

pub fn compare(input: &Input<'_>) -> Vec<AccountColumn> {
    let r_of =
        |trade: &Trade| risk::r_multiple(trade, input.reviews.get(&trade.key).and_then(|r| r.stop));
    let mut columns = Vec::new();
    for account in input.accounts {
        let mine: Vec<&Trade> = input
            .trades
            .iter()
            .copied()
            .filter(|trade| trade.account_uid == account.uid)
            .collect();
        if mine.is_empty() {
            continue;
        }
        // An account is in the comparison when its trades are in the currency asked for.
        if mine
            .iter()
            .any(|trade| currency_of(trade) != input.currency.as_deref())
        {
            continue;
        }
        let options = Options {
            initial_balance: account.initial_balance,
            breakeven_tolerance: input.breakeven_tolerance,
            offset: input.offset,
            r_of: &r_of,
        };
        let Ok(summary) = summarize(&mine, &options) else {
            continue;
        };
        let equity = equity_curve(&mine, None)
            .into_iter()
            .map(|point| (point.ts_ms, point.cum_net.to_f64().unwrap_or(0.0)))
            .collect();
        columns.push(AccountColumn {
            uid: account.uid.clone(),
            name: account.name.clone(),
            archived: account.archived,
            initial_balance: account.initial_balance,
            edge: edge_score(&summary, account.initial_balance),
            summary,
            equity,
        });
    }
    columns
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::journal::calendar::utc;
    use crate::model::journal::model::{CostBasis, Side, SourceId, TradingSourceKind};
    use crate::model::journal::reconstruct::reconstruct;
    use crate::model::journal::testing::{d, exec};

    fn account(uid: &str, balance: Option<&str>) -> JournalAccount {
        JournalAccount {
            uid: uid.into(),
            source: SourceId("t".into()),
            kind: TradingSourceKind::Paper,
            external_id: "1".into(),
            name: format!("Account {uid}"),
            currency: "USD".into(),
            initial_balance: balance.map(d),
            cost_basis: CostBasis::AverageCost,
            archived: false,
            created_at_ms: 1,
        }
    }

    fn trades() -> Vec<Trade> {
        let mut e = Vec::new();
        for (account, id, pnl, t) in [
            ("a", "1", "10", 1_000),
            ("a", "2", "-4", 3_000),
            ("b", "3", "30", 2_000),
            ("c", "4", "5", 4_000),
        ] {
            let exit = (d("100") + d(pnl)).to_string();
            e.push(exec(&format!("{id}x"), Side::Buy, "1", "100", t).in_account(account));
            e.push(exec(&format!("{id}y"), Side::Sell, "1", &exit, t + 500).in_account(account));
        }
        // Account c trades in another currency.
        for execution in &mut e {
            if execution.account_uid == "c" {
                *execution = execution.clone().currency("EUR");
            }
        }
        reconstruct(&e, |_| CostBasis::AverageCost)
    }

    fn run(accounts: &[JournalAccount], currency: Option<&str>) -> Vec<AccountColumn> {
        let trades = trades();
        let refs: Vec<&Trade> = trades.iter().collect();
        compare(&Input {
            accounts,
            trades: &refs,
            currency: currency.map(str::to_owned),
            reviews: &HashMap::new(),
            breakeven_tolerance: Decimal::ZERO,
            offset: &utc,
        })
    }

    #[test]
    fn each_account_gets_its_own_column() {
        let accounts = [
            account("a", Some("1000")),
            account("b", None),
            account("c", None),
        ];
        let columns = run(&accounts, Some("USD"));
        assert_eq!(columns.len(), 2);
        let a = &columns[0];
        assert_eq!(a.uid, "a");
        assert_eq!(a.summary.net_pnl, d("6"));
        assert_eq!(a.summary.closed, 2);
        assert_eq!(
            a.equity.iter().map(|p| p.1).collect::<Vec<_>>(),
            [10.0, 6.0]
        );
        assert_eq!(a.initial_balance, Some(d("1000")));
        assert!(a.summary.drawdown.max_pct.is_some());
        let b = &columns[1];
        assert_eq!(b.summary.net_pnl, d("30"));
        assert_eq!(b.initial_balance, None);
        assert!(b.summary.drawdown.max_pct.is_none());
    }

    #[test]
    fn another_currency_is_not_converted_or_mixed_in() {
        let accounts = [account("a", None), account("c", None)];
        let usd = run(&accounts, Some("USD"));
        assert_eq!(
            usd.iter().map(|c| c.uid.as_str()).collect::<Vec<_>>(),
            ["a"]
        );
        let eur = run(&accounts, Some("EUR"));
        assert_eq!(
            eur.iter().map(|c| c.uid.as_str()).collect::<Vec<_>>(),
            ["c"]
        );
    }

    #[test]
    fn accounts_without_trades_are_left_out() {
        let accounts = [account("a", None), account("z", None)];
        assert_eq!(run(&accounts, Some("USD")).len(), 1);
        assert!(run(&[], Some("USD")).is_empty());
    }

    #[test]
    fn a_review_stop_gives_r_per_account() {
        let accounts = [account("a", None)];
        let trades = trades();
        let refs: Vec<&Trade> = trades.iter().collect();
        let first = trades.iter().find(|t| t.account_uid == "a").unwrap();
        let mut review = Review::new(&first.key, "a");
        review.stop = Some(d("95"));
        let reviews = HashMap::from([(first.key.clone(), review)]);
        let columns = compare(&Input {
            accounts: &accounts,
            trades: &refs,
            currency: Some("USD".into()),
            reviews: &reviews,
            breakeven_tolerance: Decimal::ZERO,
            offset: &utc,
        });
        assert_eq!(columns[0].summary.r.count, 1);
    }
}
