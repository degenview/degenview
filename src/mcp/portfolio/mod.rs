//! What the portfolio tools return: portfolios, holdings valued at live prices, and the ledger.
//! Read-only. Holdings come from the app's own weighted-average accounting
//! (`model::portfolio::accounting`); this module only values them from quotes and describes them.
//!
//! The accounting is unit-naive: a holding's cost basis is in the currency its transactions were
//! priced in. The app's Portfolio tab converts between currencies with an FX service; this
//! read does not. A holding whose cost currency differs from the currency its market quotes in
//! therefore gets no profit figure (`currency_mismatch`), and totals are kept per currency
//! instead of being added across them.

pub mod tools;

use crate::mcp::chart::data::iso;
use crate::mcp::confirm::{Confirm, Gate};
use crate::model::portfolio::{
    Portfolio, PortfolioCurrency, PortfolioHolding, PortfolioTransaction, PortfolioTransactionType,
};
use crate::model::watchlist::quote::WatchlistQuote;
use crate::model::{DataSource, InstrumentId};
use schemars::JsonSchema;
use serde::Serialize;
use std::collections::BTreeMap;

/// What the portfolio tools hand the app thread, run in `host::mcp::portfolio`. All reads.
#[derive(Debug, Clone, PartialEq)]
pub enum PortfolioCommand {
    List,
    /// The ledger side of a holdings read: one portfolio, or every unarchived one.
    GetHoldings {
        portfolio_id: Option<u64>,
    },
    ListTransactions {
        portfolio_id: Option<u64>,
        asset: Option<String>,
        offset: usize,
        limit: usize,
    },
}

impl Confirm for PortfolioCommand {
    fn gate(&self) -> Gate {
        match self {
            Self::List | Self::GetHoldings { .. } | Self::ListTransactions { .. } => Gate::Free,
        }
    }
}

pub const MAX_TRANSACTION_PAGE: usize = 200;
pub const DEFAULT_TRANSACTION_PAGE: usize = 50;
/// Markets one holdings call prices at most.
pub const MAX_PRICED_HOLDINGS: usize = 100;

pub fn currency(currency: PortfolioCurrency) -> &'static str {
    match currency {
        PortfolioCurrency::Usd => "usd",
        PortfolioCurrency::Eur => "eur",
        PortfolioCurrency::Gbp => "gbp",
        PortfolioCurrency::Jpy => "jpy",
        PortfolioCurrency::Chf => "chf",
        PortfolioCurrency::Btc => "btc",
    }
}

fn kind(kind: PortfolioTransactionType) -> &'static str {
    match kind {
        PortfolioTransactionType::Buy => "buy",
        PortfolioTransactionType::Sell => "sell",
        PortfolioTransactionType::TransferIn => "transfer_in",
        PortfolioTransactionType::TransferOut => "transfer_out",
        PortfolioTransactionType::Reward => "reward",
        PortfolioTransactionType::StakingReward => "staking_reward",
        PortfolioTransactionType::Airdrop => "airdrop",
        PortfolioTransactionType::Mining => "mining",
        PortfolioTransactionType::Interest => "interest",
        PortfolioTransactionType::Fee => "fee",
        PortfolioTransactionType::Adjustment => "adjustment",
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct PortfolioInfo {
    /// Pass it to the other portfolio tools.
    pub portfolio_id: String,
    pub name: String,
    /// The currency the portfolio reports in: `usd`, `eur`, `gbp`, `jpy`, `chf` or `btc`.
    pub base_currency: String,
    pub is_archived: bool,
    pub transaction_count: usize,
    /// The portfolio the app has selected.
    pub selected: bool,
}

pub fn portfolio_info(portfolio: &Portfolio, transactions: usize, selected: bool) -> PortfolioInfo {
    PortfolioInfo {
        portfolio_id: portfolio.id.to_string(),
        name: portfolio.name.clone(),
        base_currency: currency(portfolio.base_currency).to_owned(),
        is_archived: portfolio.is_archived,
        transaction_count: transactions,
        selected,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct PortfoliosResult {
    pub portfolios: Vec<PortfolioInfo>,
    /// Whether the app is hiding amounts (privacy mode). Figures here are the real ones.
    pub privacy_mode: bool,
}

/// A holding before it is priced: what the ledger alone says.
#[derive(Debug, Clone, PartialEq)]
pub struct HoldingBase {
    pub holding: PortfolioHolding,
    /// The currency the asset's priced transactions were entered in, when they agree.
    pub cost_currency: Option<PortfolioCurrency>,
    /// The market to price it from.
    pub instrument: InstrumentId,
}

/// The ledger side of a holdings read, from the app thread.
#[derive(Debug, Clone, PartialEq)]
pub struct HoldingsBase {
    pub portfolio: Option<PortfolioInfo>,
    pub holdings: Vec<HoldingBase>,
    /// Profit and loss taken on positions since closed completely, by asset key.
    pub closed_realized: Vec<(String, f64)>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct HoldingInfo {
    /// Source-qualified asset, e.g. `BINANCE:BTCUSDT`.
    pub asset: String,
    pub source: String,
    /// The provider symbol to pass to quote, chart and watchlist tools.
    pub symbol: String,
    pub name: String,
    /// `BTC`, `AAPL`.
    pub ticker: String,
    pub quantity: f64,
    /// What the position cost, fees included, in `cost_currency`.
    pub cost_basis: f64,
    pub average_cost: f64,
    pub realized_pnl: f64,
    /// The currency the market quotes in.
    pub quote_currency: String,
    /// The currency the transactions were entered in; null when they disagree.
    pub cost_currency: Option<String>,
    /// True when the cost and quote currencies differ: price and value are shown, but profit,
    /// return and allocation are not, since converting needs an exchange rate this read lacks.
    pub currency_mismatch: bool,
    pub price: Option<f64>,
    pub previous_day_price: Option<f64>,
    pub value: Option<f64>,
    pub unrealized_pnl: Option<f64>,
    /// Unrealized profit over cost, in percent.
    pub return_percent: Option<f64>,
    pub day_change_percent: Option<f64>,
    /// Share of the priced value in the same currency, in percent.
    pub allocation_percent: Option<f64>,
    /// `live`, `stale`, `market_closed`, `unavailable` or `unsupported`.
    pub price_status: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct CurrencyTotals {
    pub currency: String,
    pub holdings: usize,
    pub cost_basis: f64,
    /// Priced holdings only.
    pub value: f64,
    pub unrealized_pnl: f64,
    pub realized_pnl: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct HoldingsResult {
    /// Null when all unarchived portfolios were read together.
    pub portfolio: Option<PortfolioInfo>,
    pub holdings: Vec<HoldingInfo>,
    /// Per currency: amounts in different currencies are never added together.
    pub totals: Vec<CurrencyTotals>,
    /// Profit taken on positions closed completely, which no holding row shows.
    pub closed_positions_realized_pnl: f64,
    /// Holdings with no current price.
    pub unpriced: usize,
    pub notes: Vec<String>,
}

fn finite(value: f64) -> Option<f64> {
    value.is_finite().then_some(value)
}

fn price_status(quote: Option<&WatchlistQuote>) -> &'static str {
    use crate::model::watchlist::quote::Freshness;
    match quote.map(|quote| quote.freshness) {
        None | Some(Freshness::Unavailable) => "unavailable",
        Some(Freshness::Live) => "live",
        Some(Freshness::Stale) => "stale",
        Some(Freshness::MarketClosed) => "market_closed",
        Some(Freshness::Unsupported) => "unsupported",
    }
}

/// Values the holdings from `quote_of` and totals them. A holding with no `last` is unpriced.
pub fn value_holdings(
    base: &HoldingsBase,
    quote_of: &dyn Fn(&InstrumentId) -> Option<WatchlistQuote>,
    priced: bool,
) -> HoldingsResult {
    let mut rows: Vec<(HoldingInfo, bool)> = Vec::new();
    for entry in &base.holdings {
        let holding = &entry.holding;
        let quote = priced.then(|| quote_of(&entry.instrument)).flatten();
        let price = quote.as_ref().and_then(|quote| quote.last).and_then(finite);
        let previous = price.and_then(|last| {
            quote
                .as_ref()
                .and_then(|quote| quote.change)
                .and_then(finite)
                .map(|change| last - change)
        });
        let quote_currency = holding.asset.quote_currency;
        let mismatch = entry.cost_currency != Some(quote_currency);
        let value = price.map(|price| price * holding.quantity);
        let unrealized = value
            .filter(|_| !mismatch)
            .map(|value| value - holding.cost_basis);
        rows.push((
            HoldingInfo {
                asset: holding.asset.key.clone(),
                source: crate::mcp::market::source_id(holding.asset.source).to_owned(),
                symbol: entry.instrument.symbol.clone(),
                name: holding
                    .asset
                    .embedded_name()
                    .map(str::to_owned)
                    .filter(|name| !name.is_empty())
                    .unwrap_or_else(|| holding.asset.name.clone()),
                ticker: holding.asset.display_ticker(),
                quantity: holding.quantity,
                cost_basis: holding.cost_basis,
                average_cost: holding.average_cost,
                realized_pnl: holding.realized_pnl,
                quote_currency: currency(quote_currency).to_owned(),
                cost_currency: entry.cost_currency.map(|c| currency(c).to_owned()),
                currency_mismatch: mismatch,
                price,
                previous_day_price: previous,
                value,
                unrealized_pnl: unrealized,
                return_percent: unrealized
                    .filter(|_| holding.cost_basis != 0.0)
                    .map(|pnl| pnl / holding.cost_basis * 100.0),
                day_change_percent: previous
                    .zip(price)
                    .filter(|(previous, _)| *previous != 0.0)
                    .map(|(previous, price)| (price - previous) / previous * 100.0),
                allocation_percent: None,
                price_status: price_status(quote.as_ref()).to_owned(),
            },
            !mismatch,
        ));
    }

    // Allocation and totals are per quote currency, over holdings whose figures are comparable.
    let mut totals: BTreeMap<String, CurrencyTotals> = BTreeMap::new();
    for (row, comparable) in &rows {
        if !comparable {
            continue;
        }
        let total = totals
            .entry(row.quote_currency.clone())
            .or_insert_with(|| CurrencyTotals {
                currency: row.quote_currency.clone(),
                holdings: 0,
                cost_basis: 0.0,
                value: 0.0,
                unrealized_pnl: 0.0,
                realized_pnl: 0.0,
            });
        total.holdings += 1;
        total.cost_basis += row.cost_basis;
        total.realized_pnl += row.realized_pnl;
        if let (Some(value), Some(pnl)) = (row.value, row.unrealized_pnl) {
            total.value += value;
            total.unrealized_pnl += pnl;
        }
    }
    for (row, comparable) in &mut rows {
        if let (true, Some(value), Some(total)) =
            (*comparable, row.value, totals.get(&row.quote_currency))
            && total.value > 0.0
        {
            row.allocation_percent = Some(value / total.value * 100.0);
        }
    }

    let mut notes = Vec::new();
    if rows.iter().any(|(row, _)| row.currency_mismatch) {
        notes.push(
            "Some holdings were bought in a different currency than their market quotes in. \
             Their price and value are shown, but profit, return and allocation are left out \
             and they are not in `totals`: converting needs an exchange rate this read does not \
             use. The Portfolio tab converts them."
                .to_owned(),
        );
    }
    if !priced {
        notes.push("Prices were not asked for; value and profit are null.".to_owned());
    }
    HoldingsResult {
        portfolio: base.portfolio.clone(),
        unpriced: rows.iter().filter(|(row, _)| row.price.is_none()).count(),
        holdings: rows.into_iter().map(|(row, _)| row).collect(),
        totals: totals.into_values().collect(),
        closed_positions_realized_pnl: base.closed_realized.iter().map(|(_, pnl)| pnl).sum(),
        notes,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct TransactionInfo {
    pub transaction_id: String,
    pub portfolio_id: String,
    pub asset: String,
    pub ticker: String,
    /// `buy`, `sell`, `transfer_in`, `transfer_out`, `reward`, `staking_reward`, `airdrop`,
    /// `mining`, `interest`, `fee` or `adjustment`.
    pub kind: String,
    pub quantity: f64,
    pub price: Option<f64>,
    pub price_currency: String,
    pub fee: f64,
    pub fee_currency: String,
    pub time: String,
    pub notes: String,
    /// `manual`, `wallet`, `exchange`, `csv` or `coinmarketcap`.
    pub source: String,
}

pub fn transaction_info(transaction: &PortfolioTransaction) -> TransactionInfo {
    use crate::model::portfolio::PortfolioSource as Source;
    TransactionInfo {
        transaction_id: transaction.id.to_string(),
        portfolio_id: transaction.portfolio_id.to_string(),
        asset: transaction.asset.key.clone(),
        ticker: transaction.asset.display_ticker(),
        kind: kind(transaction.kind).to_owned(),
        quantity: transaction.quantity,
        price: transaction.price.and_then(finite),
        price_currency: currency(transaction.price_currency).to_owned(),
        fee: transaction.fee,
        fee_currency: currency(transaction.fee_currency).to_owned(),
        time: iso(transaction.timestamp),
        notes: transaction.notes.chars().take(500).collect(),
        source: match transaction.source {
            Source::Manual => "manual",
            Source::Wallet => "wallet",
            Source::Exchange => "exchange",
            Source::Csv => "csv",
            Source::CoinMarketCap => "coinmarketcap",
        }
        .to_owned(),
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct TransactionsResult {
    /// Newest first.
    pub transactions: Vec<TransactionInfo>,
    pub offset: usize,
    /// All that match.
    pub total: usize,
    pub truncated: bool,
}

pub fn transactions_result(
    transactions: &[&PortfolioTransaction],
    offset: usize,
    limit: usize,
) -> TransactionsResult {
    let mut ordered: Vec<&&PortfolioTransaction> = transactions.iter().collect();
    ordered.sort_by(|a, b| a.timestamp.cmp(&b.timestamp).then(a.id.cmp(&b.id)));
    let page: Vec<TransactionInfo> = ordered
        .iter()
        .rev()
        .skip(offset)
        .take(limit)
        .map(|transaction| transaction_info(transaction))
        .collect();
    TransactionsResult {
        truncated: offset + page.len() < transactions.len(),
        total: transactions.len(),
        transactions: page,
        offset,
    }
}

/// The market a portfolio asset is priced from.
pub fn instrument_of(source: DataSource, key: &str, symbol: &str) -> InstrumentId {
    let market = key
        .split_once(':')
        .map(|(_, symbol)| symbol)
        .filter(|symbol| !symbol.is_empty())
        .unwrap_or(symbol);
    InstrumentId::new(source, market)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::portfolio::PortfolioAsset;
    use crate::model::watchlist::quote::Freshness;
    use std::collections::HashMap;

    fn holding(key: &str, quantity: f64, cost: f64, quote: PortfolioCurrency) -> PortfolioHolding {
        PortfolioHolding {
            asset: PortfolioAsset {
                key: key.into(),
                symbol: "BTC/USDT".into(),
                name: "Bitcoin".into(),
                source: DataSource::Binance,
                quote_currency: quote,
                metadata: HashMap::new(),
            },
            quantity,
            cost_basis: cost,
            average_cost: cost / quantity,
            realized_pnl: 5.0,
            current_price: None,
            previous_day_price: None,
            current_value: None,
            unrealized_pnl: None,
            allocation: 0.0,
            is_price_stale: false,
        }
    }

    fn base(rows: Vec<(PortfolioHolding, Option<PortfolioCurrency>)>) -> HoldingsBase {
        HoldingsBase {
            portfolio: None,
            holdings: rows
                .into_iter()
                .map(|(holding, cost_currency)| HoldingBase {
                    instrument: instrument_of(DataSource::Binance, &holding.asset.key, ""),
                    holding,
                    cost_currency,
                })
                .collect(),
            closed_realized: vec![("BINANCE:OLD".into(), -2.0)],
        }
    }

    fn live(last: f64, change: f64) -> WatchlistQuote {
        WatchlistQuote {
            last: Some(last),
            change: Some(change),
            freshness: Freshness::Live,
            ..WatchlistQuote::default()
        }
    }

    #[test]
    fn holdings_are_valued_and_allocated_within_a_currency() {
        let base = base(vec![
            (
                holding("BINANCE:BTCUSDT", 2.0, 100.0, PortfolioCurrency::Usd),
                Some(PortfolioCurrency::Usd),
            ),
            (
                holding("BINANCE:ETHUSDT", 10.0, 50.0, PortfolioCurrency::Usd),
                Some(PortfolioCurrency::Usd),
            ),
        ]);
        let quotes = |id: &InstrumentId| match id.symbol.as_str() {
            "BTCUSDT" => Some(live(60.0, 10.0)),
            "ETHUSDT" => Some(live(8.0, -2.0)),
            _ => None,
        };
        let result = value_holdings(&base, &quotes, true);
        let btc = &result.holdings[0];
        assert_eq!(btc.value, Some(120.0));
        assert_eq!(btc.unrealized_pnl, Some(20.0));
        assert_eq!(btc.return_percent, Some(20.0));
        assert_eq!(btc.previous_day_price, Some(50.0));
        assert_eq!(btc.day_change_percent, Some(20.0));
        assert_eq!(btc.allocation_percent, Some(60.0));
        assert_eq!(result.holdings[1].allocation_percent, Some(40.0));
        assert_eq!(result.totals.len(), 1);
        assert_eq!(result.totals[0].value, 200.0);
        assert_eq!(result.totals[0].cost_basis, 150.0);
        assert_eq!(result.totals[0].unrealized_pnl, 50.0);
        assert_eq!(result.totals[0].realized_pnl, 10.0);
        assert_eq!(result.closed_positions_realized_pnl, -2.0);
        assert_eq!(result.unpriced, 0);
    }

    #[test]
    fn a_holding_bought_in_another_currency_gets_no_made_up_profit() {
        let base = base(vec![
            (
                holding("BINANCE:BTCUSDT", 1.0, 100.0, PortfolioCurrency::Usd),
                Some(PortfolioCurrency::Eur),
            ),
            (
                holding("BINANCE:XYZUSDT", 1.0, 10.0, PortfolioCurrency::Usd),
                None,
            ),
        ]);
        let quotes = |_: &InstrumentId| Some(live(60.0, 0.0));
        let result = value_holdings(&base, &quotes, true);
        for row in &result.holdings {
            assert!(row.currency_mismatch, "{row:?}");
            assert_eq!(row.value, Some(60.0));
            assert_eq!(row.unrealized_pnl, None);
            assert_eq!(row.return_percent, None);
            assert_eq!(row.allocation_percent, None);
        }
        assert!(result.totals.is_empty());
        assert!(
            result
                .notes
                .iter()
                .any(|note| note.contains("different currency"))
        );
    }

    #[test]
    fn unpriced_holdings_are_counted_and_priceless_reads_say_so() {
        let base = base(vec![(
            holding("BINANCE:BTCUSDT", 1.0, 100.0, PortfolioCurrency::Usd),
            Some(PortfolioCurrency::Usd),
        )]);
        let none = |_: &InstrumentId| None;
        let result = value_holdings(&base, &none, true);
        assert_eq!(result.unpriced, 1);
        assert_eq!(result.holdings[0].price_status, "unavailable");
        assert_eq!(result.holdings[0].value, None);
        assert_eq!(result.totals[0].value, 0.0);
        let skipped = value_holdings(&base, &|_| Some(live(60.0, 0.0)), false);
        assert_eq!(skipped.holdings[0].price, None);
        assert!(
            skipped
                .notes
                .iter()
                .any(|note| note.contains("not asked for"))
        );
    }

    #[test]
    fn transactions_are_newest_first_and_page() {
        let make = |id: u64, timestamp: i64| PortfolioTransaction {
            id,
            portfolio_id: 1,
            asset: holding("BINANCE:BTCUSDT", 1.0, 1.0, PortfolioCurrency::Usd).asset,
            kind: PortfolioTransactionType::Buy,
            quantity: 1.0,
            price: Some(10.0),
            price_currency: PortfolioCurrency::Usd,
            fee: 0.0,
            fee_currency: PortfolioCurrency::Usd,
            timestamp,
            notes: String::new(),
            source: Default::default(),
            external_transaction_id: None,
            created_at: 0,
            updated_at: 0,
        };
        let all = [make(1, 100), make(2, 300), make(3, 200)];
        let refs: Vec<&PortfolioTransaction> = all.iter().collect();
        let page = transactions_result(&refs, 0, 2);
        let ids: Vec<_> = page
            .transactions
            .iter()
            .map(|t| t.transaction_id.as_str())
            .collect();
        assert_eq!(ids, ["2", "3"]);
        assert!(page.truncated);
        assert_eq!(page.total, 3);
        let rest = transactions_result(&refs, 2, 2);
        assert_eq!(rest.transactions[0].transaction_id, "1");
        assert!(!rest.truncated);
    }
}
