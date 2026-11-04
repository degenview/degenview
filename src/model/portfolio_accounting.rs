#![allow(dead_code)]
//! Pure weighted-average portfolio accounting, ported from `PortfolioAccountingEngine`.
//! Static accounting changes only with ledger edits; live valuation is applied
//! separately so quote ticks never replay transactions.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use super::portfolio::{
    PortfolioAsset, PortfolioError, PortfolioHolding, PortfolioQuote, PortfolioSource,
    PortfolioTransaction, PortfolioTransactionType,
};

struct Position {
    asset: PortfolioAsset,
    quantity: f64,
    /// The largest quantity held since the position was last flat: the scale that float error
    /// in `quantity` is measured against.
    peak: f64,
    cost_basis: f64,
    realized_pnl: f64,
}

/// Quantities are binary floats, so `0.3 - 0.1 - 0.2` is not zero. A remainder (or shortfall)
/// within this fraction of the position's peak quantity is rounding error, not holdings.
const RELATIVE_TOLERANCE: f64 = 1e-12;

impl Position {
    fn tolerance(&self) -> f64 {
        self.peak * RELATIVE_TOLERANCE
    }

    fn grow(&mut self, quantity: f64) {
        self.quantity += quantity;
        self.peak = self.peak.max(self.quantity);
    }
}

/// An asset whose position has been closed completely: nothing is held, but what the trades
/// earned still counts towards the portfolio's realized profit.
#[derive(Clone, Debug, PartialEq)]
pub struct ClosedPosition {
    pub asset: PortfolioAsset,
    pub realized_pnl: f64,
}

/// Roll transactions up into per-asset holdings, priced by `quotes`. `through` is the
/// inclusive upper bound on transaction time (None = all time). `now_ms` drives the
/// quote staleness flag.
pub fn holdings(
    transactions: &[PortfolioTransaction],
    portfolio_ids: &HashSet<u64>,
    through: Option<i64>,
    quotes: &HashMap<String, PortfolioQuote>,
    now_ms: i64,
) -> Result<Vec<PortfolioHolding>, PortfolioError> {
    holdings_and_closed(transactions, portfolio_ids, through, quotes, now_ms)
        .map(|(holdings, _)| holdings)
}

/// [`holdings`] plus the positions that were closed completely, with what they realized.
/// Realized profit of the whole portfolio is the sum over both lists.
pub fn holdings_and_closed(
    transactions: &[PortfolioTransaction],
    portfolio_ids: &HashSet<u64>,
    through: Option<i64>,
    quotes: &HashMap<String, PortfolioQuote>,
    now_ms: i64,
) -> Result<(Vec<PortfolioHolding>, Vec<ClosedPosition>), PortfolioError> {
    let mut positions: HashMap<String, Position> = HashMap::new();
    let mut ordered: Vec<&PortfolioTransaction> = transactions
        .iter()
        .filter(|tx| {
            portfolio_ids.contains(&tx.portfolio_id) && through.is_none_or(|t| tx.timestamp <= t)
        })
        .collect();
    ordered.sort_by(|a, b| transaction_order(a, b));
    for transaction in ordered {
        apply(transaction, &mut positions)?;
    }

    let (open, closed): (Vec<Position>, Vec<Position>) = positions
        .into_values()
        .partition(|position| position.quantity != 0.0);
    let mut closed: Vec<ClosedPosition> = closed
        .into_iter()
        .map(|position| ClosedPosition {
            asset: position.asset,
            realized_pnl: position.realized_pnl,
        })
        .collect();
    closed.sort_by(|a, b| a.asset.key.cmp(&b.asset.key));

    let mut holdings: Vec<PortfolioHolding> = open
        .into_iter()
        .map(|position| {
            let quote = quotes.get(&position.asset.key);
            let value = quote.map(|quote| position.quantity * quote.price);
            let unrealized = value.map(|value| value - position.cost_basis);
            PortfolioHolding {
                asset: position.asset,
                quantity: position.quantity,
                cost_basis: position.cost_basis,
                average_cost: position.cost_basis / position.quantity,
                realized_pnl: position.realized_pnl,
                current_price: quote.map(|quote| quote.price),
                previous_day_price: quote.and_then(|quote| quote.previous_day_price),
                current_value: value,
                unrealized_pnl: unrealized,
                allocation: 0.0,
                is_price_stale: quote.is_none_or(|quote| now_ms - quote.timestamp > 300_000),
            }
        })
        .collect();

    // Positions come out of a hash map: a fixed order keeps every list built from them from
    // reshuffling between frames.
    holdings.sort_by(|a, b| a.asset.key.cmp(&b.asset.key));
    let total: f64 = holdings
        .iter()
        .filter_map(|holding| holding.current_value)
        .sum();
    if total != 0.0 {
        for holding in &mut holdings {
            holding.allocation = holding.current_value.unwrap_or(0.0) / total;
        }
    }
    Ok((holdings, closed))
}

/// A ledger may contain many events for the same asset; keep the first-seen asset per key.
pub fn unique_assets(transactions: &[PortfolioTransaction]) -> Vec<PortfolioAsset> {
    let mut seen: HashMap<String, PortfolioAsset> = HashMap::new();
    for transaction in transactions {
        seen.entry(transaction.asset.key.clone())
            .or_insert_with(|| transaction.asset.clone());
    }
    seen.into_values().collect()
}

/// Validate a transaction against the ledger it will join. `replacing_id` is the id of
/// the transaction being replaced (None when adding a new one).
pub fn validate(
    transaction: &PortfolioTransaction,
    replacing_id: Option<u64>,
    transactions: &[PortfolioTransaction],
) -> Result<(), PortfolioError> {
    if transaction.quantity <= 0.0 {
        return Err(PortfolioError::InvalidQuantity);
    }
    if matches!(
        transaction.kind,
        PortfolioTransactionType::Buy | PortfolioTransactionType::Sell
    ) && transaction.price.is_none()
    {
        return Err(PortfolioError::MissingPrice);
    }
    if transaction.fee < 0.0 {
        return Err(PortfolioError::InvalidQuantity);
    }
    if transaction.price_currency != transaction.fee_currency && transaction.fee != 0.0 {
        return Err(PortfolioError::UnsupportedCurrency {
            from: transaction.fee_currency,
            to: transaction.price_currency,
        });
    }
    if transaction.source != PortfolioSource::Manual
        && let Some(external) = &transaction.external_transaction_id
    {
        let duplicate = transactions.iter().any(|tx| {
            replacing_id != Some(tx.id)
                && tx.portfolio_id == transaction.portfolio_id
                && tx.source == transaction.source
                && tx.external_transaction_id.as_deref() == Some(external.as_str())
        });
        if duplicate {
            return Err(PortfolioError::DuplicateExternalTransaction);
        }
    }
    let mut prospective: Vec<PortfolioTransaction> = transactions
        .iter()
        .filter(|tx| replacing_id != Some(tx.id) && tx.portfolio_id == transaction.portfolio_id)
        .cloned()
        .collect();
    prospective.push(transaction.clone());
    let ids: HashSet<u64> = [transaction.portfolio_id].into_iter().collect();
    holdings(&prospective, &ids, None, &HashMap::new(), 0)?;
    Ok(())
}

/// Net cash added to (or withdrawn from) a portfolio over its history.
pub fn net_contributions(
    transactions: &[PortfolioTransaction],
    portfolio_ids: &HashSet<u64>,
    through: Option<i64>,
) -> f64 {
    transactions
        .iter()
        .filter(|tx| {
            portfolio_ids.contains(&tx.portfolio_id) && through.is_none_or(|t| tx.timestamp <= t)
        })
        .fold(0.0, |result, tx| {
            let gross = tx.quantity * tx.price.unwrap_or(0.0);
            match tx.kind {
                PortfolioTransactionType::Buy => result + gross + tx.fee,
                PortfolioTransactionType::Sell => result - gross + tx.fee,
                PortfolioTransactionType::TransferIn => result + gross + tx.fee,
                PortfolioTransactionType::TransferOut => result - gross + tx.fee,
                _ => result + tx.fee,
            }
        })
}

fn apply(
    transaction: &PortfolioTransaction,
    positions: &mut HashMap<String, Position>,
) -> Result<(), PortfolioError> {
    let price = transaction.price.unwrap_or(0.0);
    let key = transaction.asset.key.clone();
    let mut position = positions.remove(&key).unwrap_or(Position {
        asset: transaction.asset.clone(),
        quantity: 0.0,
        peak: 0.0,
        cost_basis: 0.0,
        realized_pnl: 0.0,
    });
    match transaction.kind {
        PortfolioTransactionType::Buy
        | PortfolioTransactionType::TransferIn
        | PortfolioTransactionType::Reward
        | PortfolioTransactionType::StakingReward
        | PortfolioTransactionType::Airdrop
        | PortfolioTransactionType::Mining
        | PortfolioTransactionType::Interest => {
            position.grow(transaction.quantity);
            position.cost_basis += transaction.quantity * price + transaction.fee;
        }
        PortfolioTransactionType::Sell => {
            let disposed_basis = remove(transaction.quantity, &mut position)?;
            position.realized_pnl +=
                transaction.quantity * price - transaction.fee - disposed_basis;
        }
        PortfolioTransactionType::TransferOut => {
            remove(transaction.quantity, &mut position)?;
            position.cost_basis += transaction.fee;
        }
        PortfolioTransactionType::Fee => {
            let disposed_basis = remove(transaction.quantity, &mut position)?;
            position.realized_pnl -= disposed_basis + transaction.fee;
        }
        PortfolioTransactionType::Adjustment => {
            if transaction.quantity >= 0.0 {
                position.grow(transaction.quantity);
                position.cost_basis += transaction.quantity * price + transaction.fee;
            }
        }
    }
    if position.quantity == 0.0 {
        position.cost_basis = 0.0;
        position.peak = 0.0;
    }
    positions.insert(key, position);
    Ok(())
}

/// Remove `quantity` at average cost, returning the disposed basis. Removing everything that
/// is held, give or take float error, closes the position exactly.
fn remove(quantity: f64, position: &mut Position) -> Result<f64, PortfolioError> {
    let tolerance = position.tolerance();
    if position.quantity + tolerance < quantity {
        return Err(PortfolioError::InsufficientHoldings {
            available: position.quantity,
        });
    }
    if (position.quantity - quantity).abs() <= tolerance {
        let removed_basis = position.cost_basis;
        position.quantity = 0.0;
        position.cost_basis = 0.0;
        return Ok(removed_basis);
    }
    let average = position.cost_basis / position.quantity;
    let removed_basis = average * quantity;
    position.quantity -= quantity;
    position.cost_basis -= removed_basis;
    Ok(removed_basis)
}

fn transaction_order(a: &PortfolioTransaction, b: &PortfolioTransaction) -> Ordering {
    a.timestamp
        .cmp(&b.timestamp)
        .then(a.created_at.cmp(&b.created_at))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DataSource;
    use crate::model::portfolio::{
        PortfolioAsset, PortfolioCurrency, PortfolioError, PortfolioQuote, PortfolioSource,
        PortfolioTransaction, PortfolioTransactionType,
    };
    use std::collections::{HashMap, HashSet};

    fn asset(key: &str) -> PortfolioAsset {
        PortfolioAsset {
            key: key.to_owned(),
            symbol: key.to_owned(),
            name: String::new(),
            source: DataSource::Binance,
            quote_currency: PortfolioCurrency::Usd,
            metadata: HashMap::new(),
        }
    }

    fn tx(
        id: u64,
        portfolio: u64,
        key: &str,
        kind: PortfolioTransactionType,
        quantity: f64,
        price: f64,
        ts: i64,
    ) -> PortfolioTransaction {
        PortfolioTransaction {
            id,
            portfolio_id: portfolio,
            asset: asset(key),
            kind,
            quantity,
            price: Some(price),
            price_currency: PortfolioCurrency::Usd,
            fee: 0.0,
            fee_currency: PortfolioCurrency::Usd,
            timestamp: ts,
            notes: String::new(),
            source: PortfolioSource::Manual,
            external_transaction_id: None,
            created_at: ts,
            updated_at: ts,
        }
    }

    fn ids(single: u64) -> HashSet<u64> {
        [single].into_iter().collect()
    }

    fn closed_of(
        transactions: &[PortfolioTransaction],
    ) -> (Vec<PortfolioHolding>, Vec<ClosedPosition>) {
        holdings_and_closed(transactions, &ids(1), None, &HashMap::new(), 0).unwrap()
    }

    #[test]
    fn a_full_close_keeps_its_realized_pnl() {
        let transactions = vec![
            tx(1, 1, "btc", PortfolioTransactionType::Buy, 1.0, 100.0, 100),
            tx(2, 1, "btc", PortfolioTransactionType::Sell, 1.0, 150.0, 200),
        ];
        let (open, closed) = closed_of(&transactions);
        assert!(open.is_empty());
        assert_eq!(closed.len(), 1);
        assert_eq!(closed[0].asset.key, "btc");
        assert_eq!(closed[0].realized_pnl, 50.0);
        // `holdings` alone still lists open positions only.
        assert!(
            holdings(&transactions, &ids(1), None, &HashMap::new(), 0)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn a_full_close_nets_the_fee_into_realized_pnl() {
        let mut sell = tx(2, 1, "btc", PortfolioTransactionType::Sell, 1.0, 150.0, 200);
        sell.fee = 2.0;
        let (_, closed) = closed_of(&[
            tx(1, 1, "btc", PortfolioTransactionType::Buy, 1.0, 100.0, 100),
            sell,
        ]);
        assert_eq!(closed[0].realized_pnl, 48.0);
    }

    #[test]
    fn closing_and_reopening_does_not_count_the_old_profit_twice() {
        let (open, closed) = closed_of(&[
            tx(1, 1, "btc", PortfolioTransactionType::Buy, 1.0, 100.0, 100),
            tx(2, 1, "btc", PortfolioTransactionType::Sell, 1.0, 150.0, 200),
            tx(3, 1, "btc", PortfolioTransactionType::Buy, 1.0, 200.0, 300),
        ]);
        // The reopened position is one position: its realized profit lives on the holding.
        assert!(closed.is_empty());
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].realized_pnl, 50.0);
        assert_eq!(open[0].cost_basis, 200.0);
    }

    #[test]
    fn a_partial_close_stays_open_and_a_second_asset_closes_alone() {
        let (open, closed) = closed_of(&[
            tx(1, 1, "btc", PortfolioTransactionType::Buy, 2.0, 100.0, 100),
            tx(2, 1, "btc", PortfolioTransactionType::Sell, 1.0, 150.0, 200),
            tx(3, 1, "eth", PortfolioTransactionType::Buy, 4.0, 10.0, 100),
            tx(4, 1, "eth", PortfolioTransactionType::Sell, 4.0, 5.0, 200),
        ]);
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].realized_pnl, 50.0);
        assert_eq!(closed.len(), 1);
        assert_eq!(closed[0].realized_pnl, -20.0);
    }

    #[test]
    fn decimal_quantities_close_exactly() {
        // 0.3 - 0.1 is 0.19999999999999998 in binary floating point.
        let (open, closed) = closed_of(&[
            tx(1, 1, "btc", PortfolioTransactionType::Buy, 0.3, 100.0, 100),
            tx(2, 1, "btc", PortfolioTransactionType::Sell, 0.1, 100.0, 200),
            tx(3, 1, "btc", PortfolioTransactionType::Sell, 0.2, 100.0, 300),
        ]);
        assert!(open.is_empty());
        assert_eq!(closed.len(), 1);
        assert!(closed[0].realized_pnl.abs() < 1e-9);
    }

    #[test]
    fn summed_quantities_leave_no_dust_when_sold() {
        // 0.1 + 0.2 is 0.30000000000000004: selling 0.3 must still close the position.
        let (open, closed) = closed_of(&[
            tx(1, 1, "btc", PortfolioTransactionType::Buy, 0.1, 100.0, 100),
            tx(2, 1, "btc", PortfolioTransactionType::Buy, 0.2, 100.0, 200),
            tx(3, 1, "btc", PortfolioTransactionType::Sell, 0.3, 100.0, 300),
        ]);
        assert!(open.is_empty());
        assert_eq!(closed.len(), 1);
    }

    #[test]
    fn selling_more_than_held_is_still_refused() {
        let result = holdings(
            &[
                tx(1, 1, "btc", PortfolioTransactionType::Buy, 0.3, 100.0, 100),
                tx(
                    2,
                    1,
                    "btc",
                    PortfolioTransactionType::Sell,
                    0.31,
                    100.0,
                    200,
                ),
            ],
            &ids(1),
            None,
            &HashMap::new(),
            0,
        );
        assert!(matches!(
            result,
            Err(PortfolioError::InsufficientHoldings { .. })
        ));
    }

    #[test]
    fn a_tiny_real_remainder_is_not_swallowed() {
        let (open, _) = closed_of(&[
            tx(1, 1, "btc", PortfolioTransactionType::Buy, 1.0, 100.0, 100),
            tx(
                2,
                1,
                "btc",
                PortfolioTransactionType::Sell,
                0.99999999,
                100.0,
                200,
            ),
        ]);
        assert_eq!(open.len(), 1);
        assert!(open[0].quantity > 0.0);
    }

    #[test]
    fn buy_sell_realizes_pnl_and_clears_position() {
        let transactions = vec![
            tx(1, 1, "btc", PortfolioTransactionType::Buy, 2.0, 100.0, 100),
            tx(2, 1, "btc", PortfolioTransactionType::Sell, 1.0, 150.0, 200),
        ];
        let holdings = holdings(&transactions, &ids(1), None, &HashMap::new(), 0).unwrap();
        assert_eq!(holdings.len(), 1);
        assert_eq!(holdings[0].quantity, 1.0);
        assert_eq!(holdings[0].cost_basis, 100.0);
        assert_eq!(holdings[0].realized_pnl, 50.0);
    }

    #[test]
    fn sell_more_than_held_is_rejected() {
        let transactions = vec![
            tx(1, 1, "btc", PortfolioTransactionType::Buy, 1.0, 100.0, 100),
            tx(2, 1, "btc", PortfolioTransactionType::Sell, 2.0, 100.0, 200),
        ];
        assert!(matches!(
            holdings(&transactions, &ids(1), None, &HashMap::new(), 0),
            Err(PortfolioError::InsufficientHoldings { .. })
        ));
    }

    #[test]
    fn quotes_price_holdings_and_compute_allocation() {
        let transactions = vec![
            tx(1, 1, "btc", PortfolioTransactionType::Buy, 1.0, 100.0, 100),
            tx(2, 1, "eth", PortfolioTransactionType::Buy, 1.0, 100.0, 100),
        ];
        let mut quotes = HashMap::new();
        quotes.insert(
            "btc".to_owned(),
            PortfolioQuote {
                price: 200.0,
                previous_day_price: None,
                timestamp: 0,
            },
        );
        quotes.insert(
            "eth".to_owned(),
            PortfolioQuote {
                price: 100.0,
                previous_day_price: None,
                timestamp: 0,
            },
        );
        let holdings = holdings(&transactions, &ids(1), None, &quotes, 0).unwrap();
        assert_eq!(holdings.len(), 2);
        let btc = holdings.iter().find(|h| h.asset.key == "btc").unwrap();
        assert_eq!(btc.current_value, Some(200.0));
        assert_eq!(btc.unrealized_pnl, Some(100.0));
        assert_eq!(btc.allocation, 200.0 / 300.0);
        assert!(!btc.is_price_stale); // now 0, timestamp 0 → 0s old → fresh
    }

    #[test]
    fn validate_enforces_quantity_price_and_duplicates() {
        let mut base = PortfolioTransaction {
            id: 1,
            portfolio_id: 1,
            asset: asset("btc"),
            kind: PortfolioTransactionType::Buy,
            quantity: 1.0,
            price: Some(100.0),
            price_currency: PortfolioCurrency::Usd,
            fee: 0.0,
            fee_currency: PortfolioCurrency::Usd,
            timestamp: 100,
            notes: String::new(),
            source: PortfolioSource::Manual,
            external_transaction_id: None,
            created_at: 100,
            updated_at: 100,
        };
        assert!(validate(&base, None, &[]).is_ok());
        base.quantity = 0.0;
        assert!(matches!(
            validate(&base, None, &[]),
            Err(PortfolioError::InvalidQuantity)
        ));
        base.quantity = 1.0;
        base.kind = PortfolioTransactionType::Buy;
        base.price = None;
        assert!(matches!(
            validate(&base, None, &[]),
            Err(PortfolioError::MissingPrice)
        ));
    }

    #[test]
    fn net_contributions_tracks_cash_flow() {
        let transactions = vec![
            tx(1, 1, "btc", PortfolioTransactionType::Buy, 1.0, 100.0, 100),
            tx(2, 1, "btc", PortfolioTransactionType::Sell, 1.0, 150.0, 200),
        ];
        assert_eq!(
            net_contributions(&transactions, &ids(1), None),
            100.0 - 150.0
        );
    }

    #[test]
    fn holdings_come_out_in_a_fixed_order() {
        let ledger: Vec<PortfolioTransaction> = ["d", "b", "e", "a", "c"]
            .iter()
            .enumerate()
            .map(|(index, key)| {
                tx(
                    index as u64 + 1,
                    1,
                    key,
                    PortfolioTransactionType::Buy,
                    1.0,
                    10.0,
                    index as i64,
                )
            })
            .collect();
        let ids = HashSet::from([1]);
        for _ in 0..8 {
            let keys: Vec<String> = holdings(&ledger, &ids, None, &HashMap::new(), 0)
                .unwrap()
                .into_iter()
                .map(|holding| holding.asset.key)
                .collect();
            assert_eq!(keys, ["a", "b", "c", "d", "e"]);
        }
    }
}
