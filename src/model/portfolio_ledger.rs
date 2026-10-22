#![allow(dead_code)]
//! Stateful portfolio ledger: the mutation surface over `PortfolioLedgerSnapshot`,
//! delegating accounting to `portfolio_accounting`. Ported from `PortfolioLedger`.

use std::collections::{HashMap, HashSet};

use super::portfolio::{
    Portfolio, PortfolioAsset, PortfolioCurrency, PortfolioError, PortfolioHoldingsSort,
    PortfolioLedgerSnapshot, PortfolioSnapshot, PortfolioSource, PortfolioTransaction,
    normalize_name,
};
use super::portfolio_accounting::{holdings, validate};

pub struct PortfolioLedger {
    snapshot: PortfolioLedgerSnapshot,
    next_id: u64,
}

impl PortfolioLedger {
    pub fn new(snapshot: PortfolioLedgerSnapshot) -> Self {
        let next_id = snapshot
            .portfolios
            .iter()
            .map(|portfolio| portfolio.id)
            .chain(snapshot.transactions.iter().map(|tx| tx.id))
            // Ids at or above the legacy limit belong to records that arrived from another Mac.
            .filter(|id| *id < crate::sync::ids::LEGACY_LIMIT)
            .max()
            .map_or(1, |max| max + 1);
        Self { snapshot, next_id }
    }

    pub fn snapshot(&self) -> &PortfolioLedgerSnapshot {
        &self.snapshot
    }

    pub fn into_snapshot(self) -> PortfolioLedgerSnapshot {
        self.snapshot
    }

    pub fn create_portfolio(
        &mut self,
        name: &str,
        currency: PortfolioCurrency,
    ) -> Result<u64, PortfolioError> {
        let name = normalize_name(name);
        let name = if name.is_empty() {
            "Unnamed Portfolio".to_owned()
        } else {
            name
        };
        let now = now_ms();
        let id = self.mint();
        let portfolio = Portfolio {
            id,
            name,
            base_currency: currency,
            created_at: now,
            updated_at: now,
            is_archived: false,
            sort: PortfolioHoldingsSort::CurrentValue,
        };
        self.snapshot.portfolios.push(portfolio);
        self.snapshot.selected_portfolio_id = Some(id);
        Ok(id)
    }

    pub fn select(&mut self, id: Option<u64>) -> Result<(), PortfolioError> {
        if let Some(id) = id
            && !self.snapshot.portfolios.iter().any(|p| p.id == id)
        {
            return Err(PortfolioError::PortfolioNotFound);
        }
        self.snapshot.selected_portfolio_id = id;
        Ok(())
    }

    pub fn update_portfolio(&mut self, portfolio: Portfolio) -> Result<(), PortfolioError> {
        let index = self
            .snapshot
            .portfolios
            .iter()
            .position(|p| p.id == portfolio.id)
            .ok_or(PortfolioError::PortfolioNotFound)?;
        let mut value = portfolio;
        value.updated_at = now_ms();
        self.snapshot.portfolios[index] = value;
        Ok(())
    }

    pub fn duplicate_portfolio(&mut self, id: u64) -> Result<u64, PortfolioError> {
        let original = self
            .snapshot
            .portfolios
            .iter()
            .find(|p| p.id == id)
            .ok_or(PortfolioError::PortfolioNotFound)?
            .clone();
        let now = now_ms();
        let copy_id = self.mint();
        let copy = Portfolio {
            id: copy_id,
            name: format!("{} Copy", original.name),
            base_currency: original.base_currency,
            created_at: now,
            updated_at: now,
            is_archived: false,
            sort: original.sort,
        };

        // Collect sources first so the mint() calls below don't borrow the snapshot.
        let sources: Vec<PortfolioTransaction> = self
            .snapshot
            .transactions
            .iter()
            .filter(|tx| tx.portfolio_id == id)
            .cloned()
            .collect();
        let mut copied = Vec::with_capacity(sources.len());
        for tx in sources {
            copied.push(PortfolioTransaction {
                id: self.mint(),
                portfolio_id: copy_id,
                asset: tx.asset,
                kind: tx.kind,
                quantity: tx.quantity,
                price: tx.price,
                price_currency: tx.price_currency,
                fee: tx.fee,
                fee_currency: tx.fee_currency,
                timestamp: tx.timestamp,
                notes: tx.notes,
                source: PortfolioSource::Manual,
                external_transaction_id: None,
                created_at: now,
                updated_at: now,
            });
        }
        let start = copied.iter().map(|tx| tx.timestamp).min().unwrap_or(now);
        self.snapshot.portfolios.push(copy);
        self.snapshot.transactions.extend(copied);
        self.snapshot.selected_portfolio_id = Some(copy_id);
        self.invalidate(copy_id, start);
        Ok(copy_id)
    }

    pub fn reorder(&mut self, from: &[usize], destination: usize) {
        let moving: Vec<Portfolio> = from
            .iter()
            .map(|&index| self.snapshot.portfolios[index].clone())
            .collect();
        let mut descending: Vec<usize> = from.to_vec();
        descending.sort_unstable_by(|a, b| b.cmp(a));
        for index in descending {
            self.snapshot.portfolios.remove(index);
        }
        let removed_before = from.iter().filter(|&&index| index < destination).count();
        let insert_at = destination.saturating_sub(removed_before);
        self.snapshot
            .portfolios
            .splice(insert_at..insert_at, moving);
    }

    pub fn delete_portfolio(&mut self, id: u64) -> Result<(), PortfolioError> {
        if !self.snapshot.portfolios.iter().any(|p| p.id == id) {
            return Err(PortfolioError::PortfolioNotFound);
        }
        self.snapshot.portfolios.retain(|p| p.id != id);
        self.snapshot
            .transactions
            .retain(|tx| tx.portfolio_id != id);
        self.snapshot
            .historical_snapshots
            .retain(|snapshot| snapshot.portfolio_id != id);
        self.snapshot.invalidated_after.remove(&id);
        if self.snapshot.selected_portfolio_id == Some(id) {
            self.snapshot.selected_portfolio_id = self.snapshot.portfolios.first().map(|p| p.id);
        }
        Ok(())
    }

    pub fn add_transaction(
        &mut self,
        transaction: PortfolioTransaction,
    ) -> Result<(), PortfolioError> {
        if !self
            .snapshot
            .portfolios
            .iter()
            .any(|p| p.id == transaction.portfolio_id)
        {
            return Err(PortfolioError::PortfolioNotFound);
        }
        let portfolio_id = transaction.portfolio_id;
        let timestamp = transaction.timestamp;
        validate(&transaction, None, &self.snapshot.transactions)?;
        self.snapshot.transactions.push(transaction);
        self.touch(portfolio_id);
        self.invalidate(portfolio_id, timestamp);
        Ok(())
    }

    pub fn update_transaction(
        &mut self,
        transaction: PortfolioTransaction,
    ) -> Result<(), PortfolioError> {
        let index = self
            .snapshot
            .transactions
            .iter()
            .position(|tx| tx.id == transaction.id)
            .ok_or(PortfolioError::TransactionNotFound)?;
        let old = self.snapshot.transactions[index].clone();
        let portfolio_id = transaction.portfolio_id;
        let timestamp = transaction.timestamp;
        validate(
            &transaction,
            Some(transaction.id),
            &self.snapshot.transactions,
        )?;
        self.snapshot.transactions[index] = transaction;
        self.touch(portfolio_id);
        self.invalidate(old.portfolio_id, old.timestamp.min(timestamp));
        if old.portfolio_id != portfolio_id {
            self.invalidate(portfolio_id, timestamp);
        }
        Ok(())
    }

    pub fn delete_transaction(&mut self, id: u64) -> Result<(), PortfolioError> {
        let transaction = self
            .snapshot
            .transactions
            .iter()
            .find(|tx| tx.id == id)
            .ok_or(PortfolioError::TransactionNotFound)?
            .clone();
        let remaining: Vec<PortfolioTransaction> = self
            .snapshot
            .transactions
            .iter()
            .filter(|tx| tx.id != id)
            .cloned()
            .collect();
        let ids: HashSet<u64> = [transaction.portfolio_id].into_iter().collect();
        holdings(&remaining, &ids, None, &HashMap::new(), 0)?;
        self.snapshot.transactions = remaining;
        self.touch(transaction.portfolio_id);
        self.invalidate(transaction.portfolio_id, transaction.timestamp);
        Ok(())
    }

    /// Rebind every event for an asset within the requested portfolios. This changes
    /// canonical identity, so the complete prospective ledger is validated before commit.
    pub fn remap_asset(
        &mut self,
        source_key: &str,
        destination: &PortfolioAsset,
        portfolio_ids: &HashSet<u64>,
    ) -> Result<(), PortfolioError> {
        if source_key == destination.key {
            return Ok(());
        }
        let mut candidate = self.snapshot.transactions.clone();
        let mut affected: HashMap<u64, i64> = HashMap::new();
        let mut did_change = false;
        let now = now_ms();
        for tx in &mut candidate {
            if portfolio_ids.contains(&tx.portfolio_id) && tx.asset.key == source_key {
                let start = affected.entry(tx.portfolio_id).or_insert(tx.timestamp);
                *start = (*start).min(tx.timestamp);
                tx.asset = destination.clone();
                tx.updated_at = now;
                did_change = true;
            }
        }
        if !did_change {
            return Ok(());
        }
        for &portfolio_id in affected.keys() {
            let ids: HashSet<u64> = [portfolio_id].into_iter().collect();
            holdings(&candidate, &ids, None, &HashMap::new(), 0)?;
        }
        self.snapshot.transactions = candidate;
        for (&portfolio_id, &start) in &affected {
            self.touch(portfolio_id);
            self.invalidate(portfolio_id, start);
        }
        Ok(())
    }

    /// Import is all-or-nothing: validate the complete candidate ledger before committing.
    pub fn import_transactions(
        &mut self,
        values: Vec<PortfolioTransaction>,
    ) -> Result<(), PortfolioError> {
        let mut candidate = self.snapshot.transactions.clone();
        let mut chronological = values;
        chronological.sort_by(|a, b| {
            a.timestamp
                .cmp(&b.timestamp)
                .then(a.created_at.cmp(&b.created_at))
        });
        for value in &chronological {
            validate(value, None, &candidate)?;
            candidate.push(value.clone());
        }
        self.snapshot.transactions = candidate;
        for value in &chronological {
            self.touch(value.portfolio_id);
            self.invalidate(value.portfolio_id, value.timestamp);
        }
        Ok(())
    }

    pub fn store_snapshots(
        &mut self,
        snapshots: Vec<PortfolioSnapshot>,
        portfolio_id: u64,
        from: i64,
    ) -> Result<(), PortfolioError> {
        self.snapshot
            .historical_snapshots
            .retain(|snapshot| snapshot.portfolio_id != portfolio_id || snapshot.timestamp < from);
        self.snapshot.historical_snapshots.extend(snapshots);
        self.snapshot.invalidated_after.remove(&portfolio_id);
        Ok(())
    }

    fn mint(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn invalidate(&mut self, id: u64, date: i64) {
        self.snapshot
            .invalidated_after
            .entry(id)
            .and_modify(|current| *current = (*current).min(date))
            .or_insert(date);
        self.snapshot
            .historical_snapshots
            .retain(|snapshot| snapshot.portfolio_id != id || snapshot.timestamp < date);
    }

    fn touch(&mut self, id: u64) {
        if let Some(portfolio) = self.snapshot.portfolios.iter_mut().find(|p| p.id == id) {
            portfolio.updated_at = now_ms();
        }
    }
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DataSource;
    use crate::model::portfolio::{
        PortfolioAsset, PortfolioCurrency, PortfolioError, PortfolioLedgerSnapshot,
        PortfolioSource, PortfolioTransaction, PortfolioTransactionType,
    };
    use std::collections::HashMap;

    #[test]
    fn counter_ignores_ids_that_arrived_from_another_mac() {
        let mut ledger = PortfolioLedger::new(PortfolioLedgerSnapshot::default());
        ledger
            .create_portfolio("Mine", PortfolioCurrency::Usd)
            .unwrap();
        let mut snapshot = ledger.into_snapshot();
        snapshot.portfolios[0].id = crate::sync::ids::GLOBAL_BASE | (9 << 24) | 1;
        let mut ledger = PortfolioLedger::new(snapshot);
        let local = ledger
            .create_portfolio("Local", PortfolioCurrency::Usd)
            .unwrap();
        assert!(local < crate::sync::ids::LEGACY_LIMIT, "minted {local}");
    }

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

    fn transaction(
        id: u64,
        portfolio: u64,
        key: &str,
        quantity: f64,
        price: f64,
    ) -> PortfolioTransaction {
        PortfolioTransaction {
            id,
            portfolio_id: portfolio,
            asset: asset(key),
            kind: PortfolioTransactionType::Buy,
            quantity,
            price: Some(price),
            price_currency: PortfolioCurrency::Usd,
            fee: 0.0,
            fee_currency: PortfolioCurrency::Usd,
            timestamp: 1_000,
            notes: String::new(),
            source: PortfolioSource::Manual,
            external_transaction_id: None,
            created_at: 1_000,
            updated_at: 1_000,
        }
    }

    #[test]
    fn create_and_duplicate_portfolio() {
        let mut ledger = PortfolioLedger::new(PortfolioLedgerSnapshot::default());
        let id = ledger
            .create_portfolio("  Main  ", PortfolioCurrency::Usd)
            .unwrap();
        assert_eq!(ledger.snapshot().portfolios[0].name, "Main");
        assert_eq!(ledger.snapshot().selected_portfolio_id, Some(id));

        ledger
            .add_transaction(transaction(100, id, "btc", 1.0, 50.0))
            .unwrap();
        let copy = ledger.duplicate_portfolio(id).unwrap();
        assert_ne!(copy, id);
        assert_eq!(ledger.snapshot().portfolios.len(), 2);
        assert_eq!(ledger.snapshot().portfolios[1].name, "Main Copy");
        let copied_tx = ledger
            .snapshot()
            .transactions
            .iter()
            .find(|tx| tx.portfolio_id == copy)
            .unwrap();
        assert_eq!(copied_tx.source, PortfolioSource::Manual);
        assert_eq!(copied_tx.quantity, 1.0);
    }

    #[test]
    fn delete_removes_transactions_and_reselects() {
        let mut ledger = PortfolioLedger::new(PortfolioLedgerSnapshot::default());
        let first = ledger
            .create_portfolio("A", PortfolioCurrency::Usd)
            .unwrap();
        let second = ledger
            .create_portfolio("B", PortfolioCurrency::Usd)
            .unwrap();
        ledger
            .add_transaction(transaction(100, first, "btc", 1.0, 50.0))
            .unwrap();
        ledger.delete_portfolio(first).unwrap();
        assert_eq!(ledger.snapshot().portfolios.len(), 1);
        assert!(ledger.snapshot().transactions.is_empty());
        assert_eq!(ledger.snapshot().selected_portfolio_id, Some(second));
    }

    #[test]
    fn delete_transaction_revalidates_remaining_holdings() {
        let mut ledger = PortfolioLedger::new(PortfolioLedgerSnapshot::default());
        let portfolio = ledger
            .create_portfolio("A", PortfolioCurrency::Usd)
            .unwrap();
        ledger
            .add_transaction(transaction(1, portfolio, "btc", 1.0, 50.0))
            .unwrap();
        let sell = PortfolioTransaction {
            kind: PortfolioTransactionType::Sell,
            ..transaction(2, portfolio, "btc", 1.0, 60.0)
        };
        ledger.add_transaction(sell).unwrap();
        assert!(matches!(
            ledger.delete_transaction(1),
            Err(PortfolioError::InsufficientHoldings { .. })
        ));
    }

    #[test]
    fn add_rejects_unknown_portfolio_and_bad_quantity() {
        let mut ledger = PortfolioLedger::new(PortfolioLedgerSnapshot::default());
        let mut bad = transaction(1, 999, "btc", 1.0, 50.0);
        assert!(matches!(
            ledger.add_transaction(bad.clone()),
            Err(PortfolioError::PortfolioNotFound)
        ));
        let portfolio = ledger
            .create_portfolio("A", PortfolioCurrency::Usd)
            .unwrap();
        bad.portfolio_id = portfolio;
        bad.quantity = 0.0;
        assert!(matches!(
            ledger.add_transaction(bad),
            Err(PortfolioError::InvalidQuantity)
        ));
    }
}
