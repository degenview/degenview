//! What a chart shows of the journal: the trades of one market as points in time and price.
//!
//! A pure view over reconstructed [`Trade`]s, handed to charts so they can paint entries, exits,
//! the line between them and the result. Nothing here is stored or synced; prices are the real
//! fill prices (never a constructed bar's), so a mark sits at where the trade actually happened
//! whatever the chart type.

use std::collections::HashMap;
use std::sync::Arc;

use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use super::filters::{OutcomeFilter, outcome_of};
use super::format;
use super::model::{Direction, Side, Trade};

#[derive(Clone, Debug, PartialEq)]
pub struct MarkFill {
    pub ts_ms: i64,
    pub price: f64,
    pub quantity: f64,
    pub side: Side,
    pub is_entry: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TradeMark {
    /// The trade's key: what a click opens in the journal.
    pub key: String,
    pub account_uid: String,
    pub direction: Direction,
    pub outcome: OutcomeFilter,
    pub open_ts_ms: i64,
    pub close_ts_ms: Option<i64>,
    pub fills: Vec<MarkFill>,
    pub avg_entry: f64,
    pub avg_exit: Option<f64>,
    /// `+112.88`: the net result in the account's currency, only for a closed trade.
    pub label: Option<String>,
    /// The currency the label is in, for the tooltip.
    pub currency: Option<String>,
    pub quantity: Decimal,
    pub display_name: String,
}

impl TradeMark {
    /// The first and last time this trade touches, for deciding whether it is in view.
    pub fn span(&self) -> (i64, i64) {
        (
            self.open_ts_ms,
            self.close_ts_ms
                .or_else(|| self.fills.last().map(|fill| fill.ts_ms))
                .unwrap_or(self.open_ts_ms),
        )
    }
}

/// Every market's marks, ready to look up by the market's key.
#[derive(Clone, Debug, Default)]
pub struct Marks {
    by_market: HashMap<String, Arc<Vec<TradeMark>>>,
    count: usize,
}

impl Marks {
    /// Marks for `trades`, those of `account` only when one is given. Within a market the marks are
    /// ordered by when the trade opened.
    pub fn build(trades: &[Trade], account: Option<&str>, breakeven_tolerance: Decimal) -> Self {
        let mut grouped: HashMap<String, Vec<TradeMark>> = HashMap::new();
        let mut count = 0;
        for trade in trades {
            if account.is_some_and(|account| account != trade.account_uid) {
                continue;
            }
            let Some(mark) = mark_of(trade, breakeven_tolerance) else {
                continue;
            };
            grouped
                .entry(trade.instrument.market_key.clone())
                .or_default()
                .push(mark);
            count += 1;
        }
        let by_market = grouped
            .into_iter()
            .map(|(market, mut marks)| {
                marks.sort_by_key(|mark| mark.open_ts_ms);
                (market, Arc::new(marks))
            })
            .collect();
        Self { by_market, count }
    }

    /// The marks of one market (`InstrumentId::key`), if there are any.
    pub fn market(&self, market_key: &str) -> Option<Arc<Vec<TradeMark>>> {
        self.by_market.get(market_key).cloned()
    }

    pub fn len(&self) -> usize {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }
}

/// The smallest chart timeframe that shows a trade held for `hold_ms` in at most 80 bars (never
/// finer than the chart's smallest, one hour: a trade shorter than a bar sits inside one).
pub fn timeframe_for(hold_ms: i64) -> crate::model::Timeframe {
    crate::model::Timeframe::ALL
        .into_iter()
        .find(|timeframe| hold_ms / timeframe.nominal_ms() <= 80)
        .unwrap_or(crate::model::Timeframe::Year)
}

fn mark_of(trade: &Trade, tolerance: Decimal) -> Option<TradeMark> {
    let fills: Vec<MarkFill> = trade
        .legs
        .iter()
        .filter_map(|leg| {
            Some(MarkFill {
                ts_ms: leg.ts_ms,
                price: leg.price.to_f64().filter(|price| price.is_finite())?,
                quantity: leg.quantity.to_f64().unwrap_or(0.0),
                side: leg.side,
                is_entry: leg.is_entry,
            })
        })
        .collect();
    if fills.is_empty() {
        return None;
    }
    let outcome = outcome_of(trade, tolerance);
    let label = (outcome != OutcomeFilter::Open).then(|| format::signed_money(trade.net_pnl()));
    Some(TradeMark {
        key: trade.key.clone(),
        account_uid: trade.account_uid.clone(),
        direction: trade.direction,
        outcome,
        open_ts_ms: trade.open_ts_ms,
        close_ts_ms: trade.close_ts_ms,
        fills,
        avg_entry: trade.avg_entry.to_f64().filter(|price| price.is_finite())?,
        avg_exit: trade.avg_exit.and_then(|price| price.to_f64()),
        label,
        currency: trade.currency.clone(),
        quantity: trade.entry_quantity,
        display_name: trade.instrument.display_name.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::journal::model::{TradeLeg, TradeStatus};
    use crate::model::journal::testing::{d, instrument};

    fn leg(ts: i64, side: Side, price: &str, entry: bool) -> TradeLeg {
        TradeLeg {
            exec_uid: format!("e{ts}"),
            ts_ms: ts,
            side,
            quantity: d("1"),
            price: d(price),
            fee: Some(Decimal::ZERO),
            is_entry: entry,
            realized_gross: Decimal::ZERO,
            position_after: Decimal::ZERO,
        }
    }

    fn trade(key: &str, account: &str, open: i64, closed: bool, exit: &str) -> Trade {
        let mut legs = vec![leg(open, Side::Buy, "100", true)];
        let mut gross = Decimal::ZERO;
        if closed {
            legs.push(leg(open + 1_000, Side::Sell, exit, false));
            gross = d(exit) - d("100");
        }
        Trade {
            key: key.into(),
            account_uid: account.into(),
            instrument: instrument("BTCUSDT"),
            direction: Direction::Long,
            status: if closed {
                TradeStatus::Closed
            } else {
                TradeStatus::Open
            },
            open_ts_ms: open,
            close_ts_ms: closed.then_some(open + 1_000),
            legs,
            entry_quantity: d("1"),
            exit_quantity: if closed { d("1") } else { Decimal::ZERO },
            open_quantity: if closed { Decimal::ZERO } else { d("1") },
            max_quantity: d("1"),
            avg_entry: d("100"),
            avg_exit: closed.then(|| d(exit)),
            multiplier: Decimal::ONE,
            gross_pnl: gross,
            fees: Decimal::ZERO,
            fees_known: true,
            open_fees: Decimal::ZERO,
            currency: Some("USD".into()),
            initial_stop: None,
        }
    }

    #[test]
    fn marks_group_by_market_sorted_by_open_and_carry_the_result() {
        let trades = [
            trade("b", "a1", 5_000, true, "90"),
            trade("a", "a1", 1_000, true, "112.88"),
            trade("c", "a1", 9_000, false, "0"),
        ];
        let marks = Marks::build(&trades, None, Decimal::ZERO);
        assert_eq!(marks.len(), 3);
        let market = marks.market(&trades[0].instrument.market_key).unwrap();
        let keys: Vec<_> = market.iter().map(|mark| mark.key.as_str()).collect();
        assert_eq!(keys, ["a", "b", "c"]);
        assert_eq!(market[0].label.as_deref(), Some("+12.88"));
        assert_eq!(market[0].outcome, OutcomeFilter::Win);
        assert_eq!(market[1].label.as_deref(), Some("-10.00"));
        assert_eq!(market[1].outcome, OutcomeFilter::Loss);
        // An open trade has entries and no result.
        assert_eq!(market[2].label, None);
        assert_eq!(market[2].outcome, OutcomeFilter::Open);
        assert_eq!(market[2].span(), (9_000, 9_000));
        assert_eq!(market[0].span(), (1_000, 2_000));
        assert!(marks.market("BINANCE:ETHUSDT").is_none());
    }

    #[test]
    fn the_timeframe_fits_the_hold() {
        use crate::model::Timeframe;
        let hour = 3_600_000;
        assert_eq!(timeframe_for(0), Timeframe::Hour);
        assert_eq!(timeframe_for(80 * hour), Timeframe::Hour);
        assert_eq!(timeframe_for(81 * hour), Timeframe::Day);
        assert_eq!(timeframe_for(60 * 24 * hour), Timeframe::Day);
        assert_eq!(timeframe_for(200 * 24 * hour), Timeframe::Week);
        assert_eq!(timeframe_for(i64::MAX), Timeframe::Year);
    }

    #[test]
    fn the_account_choice_limits_the_marks() {
        let trades = [
            trade("a", "a1", 1_000, true, "110"),
            trade("b", "a2", 2_000, true, "110"),
        ];
        let only = Marks::build(&trades, Some("a2"), Decimal::ZERO);
        let market = only.market(&trades[0].instrument.market_key).unwrap();
        assert_eq!(market.len(), 1);
        assert_eq!(market[0].key, "b");
        assert!(Marks::build(&trades, Some("none"), Decimal::ZERO).is_empty());
    }
}
