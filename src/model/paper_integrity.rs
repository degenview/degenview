//! Consistency checks over a paper-trading snapshot.
//!
//! Decoding proves each row has the right shape; it does not prove the rows agree with each other.
//! Bundles cross versions and Macs as JSON, and one dangling reference or duplicate id would skew
//! every later fill and margin figure, so the host runs these after a load and before every save.
//! They are structural only: whether the cash balance equals the ledger is not checked, because an
//! older or imported account may legitimately differ and refusing to open it would cost the user
//! the account.

use std::collections::HashSet;

use super::paper::{
    PaperAccount, PaperCommissionConfiguration, PaperInstrument, PaperTradingSnapshot,
};

/// Issues past this many add nothing a person can act on.
const MAX_ISSUES: usize = 12;

impl PaperTradingSnapshot {
    /// What is wrong with the snapshot, one line each; empty when it is sound.
    pub fn integrity_issues(&self) -> Vec<String> {
        let mut issues = Vec::new();
        let mut note = |issue: String| {
            if issues.len() < MAX_ISSUES {
                issues.push(issue);
            }
        };

        let accounts: HashSet<u64> = self.accounts.iter().map(|account| account.id).collect();
        let order_ids: HashSet<u64> = self.orders.iter().map(|order| order.id).collect();
        duplicates("account", self.accounts.iter().map(|a| a.id), &mut note);
        duplicates("order", self.orders.iter().map(|o| o.id), &mut note);
        duplicates("fill", self.fills.iter().map(|f| f.id), &mut note);
        duplicates(
            "order event",
            self.order_events.iter().map(|e| e.id),
            &mut note,
        );
        duplicates(
            "closed trade",
            self.closed_trades.iter().map(|t| t.id),
            &mut note,
        );
        duplicates(
            "journal entry",
            self.journal.iter().map(|j| j.id),
            &mut note,
        );

        for account in &self.accounts {
            account_issues(account, &mut note);
        }

        for order in &self.orders {
            if !accounts.contains(&order.account_id) {
                note(format!(
                    "order {} belongs to missing account {}",
                    order.id, order.account_id
                ));
            }
            if let Some(parent) = order.parent_order_id
                && !order_ids.contains(&parent)
            {
                note(format!("order {} has a missing parent {parent}", order.id));
            }
            let quantities = [order.original_quantity, order.filled_quantity];
            if !quantities.iter().all(|q| q.is_finite() && *q >= 0.0) {
                note(format!("order {} has an invalid quantity", order.id));
            } else if order.filled_quantity > order.original_quantity + 1e-9 {
                note(format!(
                    "order {} is filled beyond its quantity ({} of {})",
                    order.id, order.filled_quantity, order.original_quantity
                ));
            }
            let prices = [
                order.limit_price,
                order.stop_price,
                order.average_fill_price,
            ];
            if prices
                .iter()
                .flatten()
                .any(|price| !price.is_finite() || *price <= 0.0)
            {
                note(format!("order {} has an invalid price", order.id));
            }
            if !order.reserved_margin.is_finite() || order.reserved_margin < 0.0 {
                note(format!("order {} has an invalid reserved margin", order.id));
            }
            instrument_issues(&order.instrument, &format!("order {}", order.id), &mut note);
        }

        for fill in &self.fills {
            if !accounts.contains(&fill.account_id) {
                note(format!("fill {} belongs to missing account", fill.id));
            }
            if !order_ids.contains(&fill.order_id) {
                note(format!(
                    "fill {} has a missing order {}",
                    fill.id, fill.order_id
                ));
            }
            let sound = fill.quantity.is_finite()
                && fill.quantity > 0.0
                && fill.price.is_finite()
                && fill.price > 0.0
                && fill.commission.is_finite()
                && fill.commission >= 0.0;
            if !sound {
                note(format!(
                    "fill {} has an invalid quantity, price or commission",
                    fill.id
                ));
            }
        }

        let mut held = HashSet::new();
        for position in &self.positions {
            if !accounts.contains(&position.account_id) {
                note(format!(
                    "position in {} belongs to missing account {}",
                    position.instrument.key, position.account_id
                ));
            }
            if !held.insert((position.account_id, position.instrument.key.as_str())) {
                note(format!("duplicate position in {}", position.instrument.key));
            }
            if !position.signed_quantity.is_finite()
                || position.signed_quantity == 0.0
                || !position.average_entry_price.is_finite()
            {
                note(format!(
                    "position in {} is invalid",
                    position.instrument.key
                ));
            }
            instrument_issues(
                &position.instrument,
                &format!("position in {}", position.instrument.key),
                &mut note,
            );
        }

        let rows = [
            (
                "order event",
                self.order_events
                    .iter()
                    .map(|e| e.account_id)
                    .collect::<Vec<_>>(),
            ),
            (
                "closed trade",
                self.closed_trades.iter().map(|t| t.account_id).collect(),
            ),
            (
                "journal entry",
                self.journal.iter().map(|j| j.account_id).collect(),
            ),
        ];
        for (what, owners) in rows {
            let missing = owners.iter().filter(|id| !accounts.contains(id)).count();
            if missing > 0 {
                note(format!("{missing} {what}(s) belong to a missing account"));
            }
        }
        issues
    }
}

fn duplicates(what: &str, ids: impl Iterator<Item = u64>, note: &mut impl FnMut(String)) {
    let mut seen = HashSet::new();
    for id in ids {
        if !seen.insert(id) {
            note(format!("duplicate {what} id {id}"));
        }
    }
}

fn account_issues(account: &PaperAccount, note: &mut impl FnMut(String)) {
    let balances = [
        account.initial_balance,
        account.cash_balance,
        account.realized_pnl,
    ];
    if !balances.iter().all(|value| value.is_finite()) {
        note(format!("account {} has a non-finite balance", account.id));
    }
    let settings = &account.settings;
    let leverage = settings.leverage;
    let leverages = [
        leverage.stocks,
        leverage.crypto,
        leverage.forex,
        leverage.futures,
        leverage.prediction,
    ];
    if !leverages
        .iter()
        .all(|value| value.is_finite() && *value > 0.0)
    {
        note(format!("account {} has an invalid leverage", account.id));
    }
    if !settings.slippage_ticks.is_finite() || settings.slippage_ticks < 0.0 {
        note(format!("account {} has an invalid slippage", account.id));
    }
    let fee = match settings.commission {
        PaperCommissionConfiguration::None => 0.0,
        PaperCommissionConfiguration::FixedPerOrder { value }
        | PaperCommissionConfiguration::Percentage { value }
        | PaperCommissionConfiguration::PerContract { value } => value,
    };
    if !fee.is_finite() || fee < 0.0 {
        note(format!("account {} has an invalid commission", account.id));
    }
}

fn instrument_issues(instrument: &PaperInstrument, owner: &str, note: &mut impl FnMut(String)) {
    let steps = [
        instrument.tick_size,
        instrument.minimum_quantity,
        instrument.quantity_increment,
    ];
    let scales = [instrument.contract_multiplier, instrument.point_value];
    if !steps.iter().all(|value| value.is_finite() && *value >= 0.0)
        || !scales.iter().all(|value| value.is_finite() && *value > 0.0)
    {
        note(format!(
            "{owner} has an invalid instrument ({})",
            instrument.key
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DataSource;
    use crate::model::paper::{
        PaperAccountSettings, PaperCurrency, PaperOrderRequest, PaperOrderSide, PaperOrderType,
        PaperQuote, PaperTimeInForce,
    };
    use crate::model::paper_engine::PaperTradingEngine;

    /// A snapshot the engine built: one account, a filled order, a position and a working order.
    fn sound() -> PaperTradingSnapshot {
        let mut engine = PaperTradingEngine::new(PaperTradingSnapshot::default());
        let account = engine
            .create_account(
                "T",
                PaperCurrency::Usd,
                100_000.0,
                PaperAccountSettings::default(),
            )
            .unwrap();
        engine
            .process(PaperQuote {
                instrument_key: "Binance:BTCUSDT".to_owned(),
                bid: Some(99.5),
                ask: Some(100.5),
                last: Some(100.0),
                timestamp: crate::model::watchlist::now_ms(),
                is_market_open: true,
            })
            .unwrap();
        let request = |kind, limit| PaperOrderRequest {
            account_id: account,
            instrument: PaperInstrument::chart("BTCUSDT", "BTC", DataSource::Binance),
            side: PaperOrderSide::Buy,
            kind,
            quantity: 1.0,
            limit_price: limit,
            stop_price: None,
            time_in_force: PaperTimeInForce::GoodTilCanceled,
            take_profit: Some(120.0),
            stop_loss: Some(50.0),
        };
        engine
            .submit(request(PaperOrderType::Market, None))
            .unwrap();
        engine
            .submit(request(PaperOrderType::Limit, Some(10.0)))
            .unwrap();
        engine.into_snapshot()
    }

    #[test]
    fn what_the_engine_builds_is_sound() {
        let snapshot = sound();
        assert!(!snapshot.orders.is_empty() && !snapshot.positions.is_empty());
        assert_eq!(snapshot.integrity_issues(), Vec::<String>::new());
        assert!(
            PaperTradingSnapshot::default()
                .integrity_issues()
                .is_empty()
        );
    }

    fn assert_flags(snapshot: &PaperTradingSnapshot, needle: &str) {
        let issues = snapshot.integrity_issues();
        assert!(
            issues.iter().any(|issue| issue.contains(needle)),
            "{needle:?} not in {issues:?}"
        );
    }

    #[test]
    fn each_broken_relationship_is_named() {
        let mut duplicate = sound();
        let first = duplicate.accounts[0].clone();
        duplicate.accounts.push(first);
        assert_flags(&duplicate, "duplicate account id");

        let mut orphan = sound();
        orphan.orders[0].account_id = 999;
        assert_flags(&orphan, "missing account 999");

        let mut overfilled = sound();
        overfilled.orders[0].filled_quantity = 5.0;
        assert_flags(&overfilled, "filled beyond");

        let mut parentless = sound();
        let child = parentless
            .orders
            .iter()
            .position(|order| order.parent_order_id.is_some())
            .unwrap();
        parentless.orders[child].parent_order_id = Some(424_242);
        assert_flags(&parentless, "missing parent");

        let mut twice = sound();
        let position = twice.positions[0].clone();
        twice.positions.push(position);
        assert_flags(&twice, "duplicate position");

        let mut nan = sound();
        nan.accounts[0].cash_balance = f64::NAN;
        assert_flags(&nan, "non-finite balance");

        let mut tick = sound();
        tick.orders[0].instrument.tick_size = f64::INFINITY;
        assert_flags(&tick, "invalid instrument");

        let mut fill = sound();
        fill.fills[0].order_id = 777_777;
        assert_flags(&fill, "missing order");
    }

    #[test]
    fn a_thousand_orders_are_checked_in_a_blink() {
        let mut snapshot = sound();
        let template = snapshot.orders[0].clone();
        for index in 0..5_000u64 {
            let mut order = template.clone();
            order.id = 1_000_000 + index;
            snapshot.orders.push(order);
        }
        let started = std::time::Instant::now();
        assert!(snapshot.integrity_issues().is_empty());
        assert!(
            started.elapsed().as_millis() < 250,
            "{:?}",
            started.elapsed()
        );
    }
}
