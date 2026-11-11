#![allow(dead_code)]
//! Portfolio data model: currencies, sources, transaction types, assets, portfolios,
//! transactions, holdings, snapshots and quotes. Mirrors `PortfolioModels.swift` with
//! Swift's `Decimal` mapped to `f64`, `UUID` to `u64` and `Date` to epoch milliseconds.

pub mod accounting;
pub mod card;
pub mod csv;
pub mod editor;
pub mod format;
pub mod history;
pub mod ledger;
pub mod statistics;

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::model::DataSource;

/// The currency a portfolio or transaction is denominated in.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PortfolioCurrency {
    #[default]
    Usd,
    Eur,
    Gbp,
    Jpy,
    Chf,
    Btc,
}

impl PortfolioCurrency {
    /// The currency a pair's quote asset is priced in. Dollar stablecoins count as USD;
    /// a quote this app can't convert falls back to USD.
    pub fn from_quote_symbol(quote: Option<&str>) -> Self {
        let quote = quote.map(str::to_uppercase);
        match quote.as_deref() {
            Some("USDT" | "USDC") => Self::Usd,
            Some("EUR") => Self::Eur,
            Some("GBP") => Self::Gbp,
            Some("JPY") => Self::Jpy,
            Some("CHF") => Self::Chf,
            Some("BTC") => Self::Btc,
            _ => Self::Usd,
        }
    }

    /// The currency's sign, for badges: `$`, `€`, `£`, `¥`, `Fr`, `₿`.
    pub fn glyph(self) -> &'static str {
        match self {
            Self::Usd => "$",
            Self::Eur => "€",
            Self::Gbp => "£",
            Self::Jpy => "¥",
            Self::Chf => "Fr",
            Self::Btc => "₿",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Usd => "US Dollar",
            Self::Eur => "Euro",
            Self::Gbp => "British Pound",
            Self::Jpy => "Japanese Yen",
            Self::Chf => "Swiss Franc",
            Self::Btc => "Bitcoin",
        }
    }
}

/// Where a transaction came from.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PortfolioSource {
    #[default]
    Manual,
    Wallet,
    Exchange,
    Csv,
    CoinMarketCap,
}

/// What a transaction does to a holding's quantity.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PortfolioTransactionType {
    Buy,
    Sell,
    TransferIn,
    TransferOut,
    Reward,
    StakingReward,
    Airdrop,
    Mining,
    Interest,
    Fee,
    #[default]
    Adjustment,
}

impl PortfolioTransactionType {
    pub fn adds_quantity(self) -> bool {
        matches!(
            self,
            Self::Buy
                | Self::TransferIn
                | Self::Reward
                | Self::StakingReward
                | Self::Airdrop
                | Self::Mining
                | Self::Interest
        )
    }

    pub fn removes_quantity(self) -> bool {
        matches!(self, Self::Sell | Self::TransferOut | Self::Fee)
    }
}

/// A source-qualified identifier. Symbols are display data and are never identity.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PortfolioAsset {
    /// Stable identity: `"<source>:<symbol>"`.
    pub key: String,
    #[serde(default)]
    pub symbol: String,
    #[serde(default)]
    pub name: String,
    pub source: DataSource,
    #[serde(default)]
    pub quote_currency: PortfolioCurrency,
    #[serde(default)]
    pub metadata: HashMap<String, String>,
}

/// Separates an equity ticker from its company name in a search label ("AAPL — Apple Inc.").
const NAME_SEPARATOR: &str = " — ";

impl PortfolioAsset {
    /// The ticker alone, without trading pair or company name: `BTC/USDT` → `BTC`.
    pub fn display_ticker(&self) -> String {
        let label = self
            .symbol
            .split(NAME_SEPARATOR)
            .next()
            .unwrap_or(&self.symbol);
        let base = label.split('/').next().unwrap_or(label);
        base.trim().to_uppercase()
    }

    /// A name the symbol itself carries (Alpaca labels equities "AAPL — Apple Inc."), if any.
    pub fn embedded_name(&self) -> Option<&str> {
        let name = self.symbol.split(NAME_SEPARATOR).nth(1)?.trim();
        if name.is_empty() { None } else { Some(name) }
    }
}

/// How a portfolio sorts its holdings table.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PortfolioHoldingsSort {
    #[default]
    CurrentValue,
    Allocation,
    DayChange,
    ProfitLoss,
    ProfitLossPercent,
    Asset,
}

/// A user portfolio: a named set of transactions in one base currency.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Portfolio {
    pub id: u64,
    pub name: String,
    #[serde(default)]
    pub base_currency: PortfolioCurrency,
    /// Epoch milliseconds.
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
    #[serde(default)]
    pub is_archived: bool,
    #[serde(default)]
    pub sort: PortfolioHoldingsSort,
}

impl Portfolio {
    pub fn new(id: u64, name: impl Into<String>, created_at: i64) -> Self {
        Self {
            id,
            name: name.into(),
            base_currency: PortfolioCurrency::Usd,
            created_at,
            updated_at: created_at,
            is_archived: false,
            sort: PortfolioHoldingsSort::CurrentValue,
        }
    }
}

/// One entry in a portfolio's ledger.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PortfolioTransaction {
    pub id: u64,
    pub portfolio_id: u64,
    pub asset: PortfolioAsset,
    #[serde(default)]
    pub kind: PortfolioTransactionType,
    pub quantity: f64,
    #[serde(default)]
    pub price: Option<f64>,
    #[serde(default)]
    pub price_currency: PortfolioCurrency,
    #[serde(default)]
    pub fee: f64,
    #[serde(default)]
    pub fee_currency: PortfolioCurrency,
    /// Epoch milliseconds.
    #[serde(default)]
    pub timestamp: i64,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub source: PortfolioSource,
    #[serde(default)]
    pub external_transaction_id: Option<String>,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
}

/// A holding computed from a portfolio's transactions, priced by a quote. Not persisted.
#[derive(Clone, Debug, PartialEq)]
pub struct PortfolioHolding {
    pub asset: PortfolioAsset,
    pub quantity: f64,
    pub cost_basis: f64,
    pub average_cost: f64,
    pub realized_pnl: f64,
    pub current_price: Option<f64>,
    pub previous_day_price: Option<f64>,
    pub current_value: Option<f64>,
    pub unrealized_pnl: Option<f64>,
    /// Fraction of the portfolio's total value, 0…=1.
    pub allocation: f64,
    pub is_price_stale: bool,
}

impl PortfolioHolding {
    pub fn total_pnl(&self) -> Option<f64> {
        self.unrealized_pnl
            .map(|unrealized| unrealized + self.realized_pnl)
    }

    pub fn pnl_percent(&self) -> Option<f64> {
        if self.cost_basis == 0.0 {
            None
        } else {
            self.unrealized_pnl.map(|pnl| pnl / self.cost_basis)
        }
    }

    pub fn day_change_percent(&self) -> Option<f64> {
        let (current, previous) = (self.current_price?, self.previous_day_price?);
        if previous == 0.0 {
            None
        } else {
            Some((current - previous) / previous)
        }
    }
}

/// A point-in-time valuation of a portfolio, for the history chart.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PortfolioSnapshot {
    pub portfolio_id: u64,
    /// Epoch milliseconds.
    pub timestamp: i64,
    pub value: f64,
    pub net_contributions: f64,
    pub realized_pnl: f64,
    pub unrealized_pnl: f64,
    pub is_complete: bool,
}

/// The persisted portfolio store contents, one document per table.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct PortfolioLedgerSnapshot {
    #[serde(default)]
    pub portfolios: Vec<Portfolio>,
    #[serde(default)]
    pub transactions: Vec<PortfolioTransaction>,
    #[serde(default)]
    pub historical_snapshots: Vec<PortfolioSnapshot>,
    #[serde(default)]
    pub selected_portfolio_id: Option<u64>,
    #[serde(default)]
    pub invalidated_after: HashMap<u64, i64>,
}

/// A market price for one asset, fetched live.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PortfolioQuote {
    pub price: f64,
    #[serde(default)]
    pub previous_day_price: Option<f64>,
    /// Epoch milliseconds.
    pub timestamp: i64,
}

/// Which portfolio a UI surfaces.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PortfolioSelection {
    All,
    Portfolio(u64),
}

/// Portfolio validation and accounting failures.
#[derive(Clone, Debug, PartialEq)]
pub enum PortfolioError {
    PortfolioNotFound,
    TransactionNotFound,
    InvalidQuantity,
    MissingPrice,
    InsufficientHoldings {
        available: f64,
    },
    UnsupportedCurrency {
        from: PortfolioCurrency,
        to: PortfolioCurrency,
    },
    DuplicateExternalTransaction,
    InvalidCsv(String),
    /// A negative or non-finite price.
    InvalidPrice,
    /// A transaction with no asset to trade.
    InvalidAsset,
}

impl PortfolioError {
    pub fn message(&self) -> String {
        match self {
            Self::PortfolioNotFound => "Portfolio not found.".to_owned(),
            Self::TransactionNotFound => "Transaction not found.".to_owned(),
            Self::InvalidQuantity => "Quantity must be greater than zero.".to_owned(),
            Self::MissingPrice => "A price is required for this transaction type.".to_owned(),
            Self::InsufficientHoldings { available } => {
                format!("Insufficient holdings at that date (available: {available}).")
            }
            Self::UnsupportedCurrency { from, to } => format!(
                "Historical conversion from {} to {} is unavailable.",
                from.display_name(),
                to.display_name()
            ),
            Self::DuplicateExternalTransaction => {
                "This imported transaction already exists.".to_owned()
            }
            Self::InvalidCsv(message) => message.clone(),
            Self::InvalidPrice => "The price cannot be negative.".to_owned(),
            Self::InvalidAsset => "The transaction has no asset.".to_owned(),
        }
    }
}

/// Rules for the name typed into the create-portfolio sheet.
pub const MAX_NAME_LENGTH: usize = 40;

pub fn normalize_name(name: &str) -> String {
    name.trim().chars().take(MAX_NAME_LENGTH).collect()
}

pub fn is_valid_name(name: &str) -> bool {
    !normalize_name(name).is_empty()
}

/// Whether another portfolio already has this name (ignoring case and surrounding spaces).
pub fn is_duplicate_name(name: &str, existing: &[String]) -> bool {
    let wanted = normalize_name(name);
    if wanted.is_empty() {
        return false;
    }
    existing
        .iter()
        .any(|other| normalize_name(other).eq_ignore_ascii_case(&wanted))
}

/// "Main Copy", then "Main Copy 2", and so on: a name no portfolio in `existing` has, and
/// short enough to be kept whole (a long name gives up its end, not the suffix).
pub fn unique_copy_name(base: &str, existing: &[String]) -> String {
    let base = normalize_name(base);
    (1u32..)
        .map(|number| {
            let suffix = if number == 1 {
                " Copy".to_owned()
            } else {
                format!(" Copy {number}")
            };
            let room = MAX_NAME_LENGTH.saturating_sub(suffix.chars().count());
            let stem: String = base.chars().take(room).collect();
            format!("{}{suffix}", stem.trim_end())
        })
        .find(|candidate| !is_duplicate_name(candidate, existing))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copies_get_a_free_name_that_fits() {
        let taken = |names: &[&str]| names.iter().map(|n| n.to_string()).collect::<Vec<_>>();
        assert_eq!(unique_copy_name("Main", &taken(&["Main"])), "Main Copy");
        assert_eq!(
            unique_copy_name("Main", &taken(&["Main", "main copy"])),
            "Main Copy 2"
        );
        assert_eq!(
            unique_copy_name("Main", &taken(&["Main", "Main Copy", "Main Copy 2"])),
            "Main Copy 3"
        );
        let long = "x".repeat(MAX_NAME_LENGTH);
        let copy = unique_copy_name(&long, std::slice::from_ref(&long));
        assert_eq!(copy.chars().count(), MAX_NAME_LENGTH);
        assert!(copy.ends_with(" Copy"));
        let second = unique_copy_name(&long, &[long.clone(), copy]);
        assert!(second.chars().count() <= MAX_NAME_LENGTH);
        assert!(second.ends_with(" Copy 2"));
    }

    #[test]
    fn currency_quote_symbol_mapping() {
        assert_eq!(
            PortfolioCurrency::from_quote_symbol(Some("USDT")),
            PortfolioCurrency::Usd
        );
        assert_eq!(
            PortfolioCurrency::from_quote_symbol(Some("usdc")),
            PortfolioCurrency::Usd
        );
        assert_eq!(
            PortfolioCurrency::from_quote_symbol(Some("EUR")),
            PortfolioCurrency::Eur
        );
        assert_eq!(
            PortfolioCurrency::from_quote_symbol(Some("TRY")),
            PortfolioCurrency::Usd
        );
        assert_eq!(
            PortfolioCurrency::from_quote_symbol(None),
            PortfolioCurrency::Usd
        );
    }

    #[test]
    fn transaction_kinds_partition_quantity() {
        assert!(PortfolioTransactionType::Buy.adds_quantity());
        assert!(PortfolioTransactionType::Airdrop.adds_quantity());
        assert!(!PortfolioTransactionType::Adjustment.adds_quantity());
        assert!(PortfolioTransactionType::Sell.removes_quantity());
        assert!(!PortfolioTransactionType::Interest.removes_quantity());
    }

    #[test]
    fn asset_display_ticker_and_embedded_name() {
        let asset = |symbol: &str| PortfolioAsset {
            key: "alpaca:aapl".into(),
            symbol: symbol.into(),
            name: String::new(),
            source: DataSource::Alpaca,
            quote_currency: PortfolioCurrency::Usd,
            metadata: HashMap::new(),
        };
        assert_eq!(asset("BTC/USDT").display_ticker(), "BTC");
        assert_eq!(asset("AAPL — Apple Inc.").display_ticker(), "AAPL");
        assert_eq!(
            asset("AAPL — Apple Inc.").embedded_name(),
            Some("Apple Inc.")
        );
        assert_eq!(asset("BTCUSDT").embedded_name(), None);
    }

    #[test]
    fn holding_computed_metrics() {
        let holding = PortfolioHolding {
            asset: PortfolioAsset {
                key: "binance:btc".into(),
                symbol: "BTC".into(),
                name: "Bitcoin".into(),
                source: DataSource::Binance,
                quote_currency: PortfolioCurrency::Usd,
                metadata: HashMap::new(),
            },
            quantity: 1.0,
            cost_basis: 1_000.0,
            average_cost: 1_000.0,
            realized_pnl: 50.0,
            current_price: Some(1_200.0),
            previous_day_price: Some(1_000.0),
            current_value: Some(1_200.0),
            unrealized_pnl: Some(200.0),
            allocation: 0.5,
            is_price_stale: false,
        };
        assert_eq!(holding.total_pnl(), Some(250.0));
        assert_eq!(holding.pnl_percent(), Some(0.2));
        assert_eq!(holding.day_change_percent(), Some(0.2));
    }

    #[test]
    fn name_rules() {
        assert_eq!(normalize_name("  My Portfolio "), "My Portfolio");
        assert!(is_valid_name("  x "));
        assert!(!is_valid_name("   "));
        assert!(is_duplicate_name("MY PORTFOLIO", &["my portfolio".into()]));
        assert!(!is_duplicate_name("  ", &["anything".into()]));
    }

    #[test]
    fn ledger_snapshot_decodes_old_rows() {
        // An old row missing every optional field still decodes to a usable snapshot.
        let snapshot: PortfolioLedgerSnapshot = serde_json::from_str("{}").unwrap();
        assert!(snapshot.portfolios.is_empty());
        assert_eq!(snapshot.selected_portfolio_id, None);
    }
}
