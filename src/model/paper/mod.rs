#![allow(dead_code)]
//! Paper-trading data model: accounts, orders, fills, positions, quotes and the
//! snapshot that persists them. Mirrors `PaperTradingModels.swift` with `Decimal` mapped
//! to `f64`, `UUID` to `u64` and `Date` to epoch milliseconds.

pub mod engine;
pub mod export;
pub mod format;
pub mod integrity;
pub mod ticket;

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::model::DataSource;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PaperCurrency {
    #[default]
    Usd,
    Eur,
    Gbp,
    Jpy,
    Usdt,
    Usdc,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PaperAssetClass {
    #[default]
    Stock,
    Crypto,
    Forex,
    Futures,
    Prediction,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PaperOrderSide {
    #[default]
    Buy,
    Sell,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PaperOrderType {
    #[default]
    Market,
    Limit,
    Stop,
    StopLimit,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PaperTimeInForce {
    Day,
    #[default]
    GoodTilCanceled,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PaperOrderStatus {
    #[default]
    PendingSubmission,
    Working,
    PartiallyFilled,
    Filled,
    PendingCancel,
    Canceled,
    Rejected,
    Expired,
}

impl PaperOrderStatus {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Filled | Self::Canceled | Self::Rejected | Self::Expired
        )
    }

    pub fn is_working(self) -> bool {
        matches!(self, Self::Working | Self::PartiallyFilled)
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PaperOrderRole {
    #[default]
    Entry,
    TakeProfit,
    StopLoss,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PaperPositionSide {
    #[default]
    Long,
    Short,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PaperFillPriceSource {
    #[default]
    BidAsk,
    LastPriceFallback,
    BarFallback,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PaperCommissionConfiguration {
    #[default]
    None,
    FixedPerOrder {
        value: f64,
    },
    Percentage {
        value: f64,
    },
    PerContract {
        value: f64,
    },
}

impl PaperCommissionConfiguration {
    pub fn label(self) -> String {
        match self {
            Self::None => "None".to_owned(),
            Self::FixedPerOrder { value } => format!("{value} per order"),
            Self::Percentage { value } => format!("{value}% of value"),
            Self::PerContract { value } => format!("{value} per contract"),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct PaperLeverageConfiguration {
    #[serde(default = "one")]
    pub stocks: f64,
    #[serde(default = "one")]
    pub crypto: f64,
    #[serde(default = "one")]
    pub forex: f64,
    #[serde(default = "one")]
    pub futures: f64,
    #[serde(default = "one")]
    pub prediction: f64,
}

fn one() -> f64 {
    1.0
}

impl Default for PaperLeverageConfiguration {
    fn default() -> Self {
        Self {
            stocks: 1.0,
            crypto: 1.0,
            forex: 1.0,
            futures: 1.0,
            prediction: 1.0,
        }
    }
}

impl PaperLeverageConfiguration {
    pub fn leverage(self, asset_class: PaperAssetClass) -> f64 {
        match asset_class {
            PaperAssetClass::Stock => self.stocks,
            PaperAssetClass::Crypto => self.crypto,
            PaperAssetClass::Forex => self.forex,
            PaperAssetClass::Futures => self.futures,
            PaperAssetClass::Prediction => self.prediction,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PaperAccountSettings {
    #[serde(default)]
    pub commission: PaperCommissionConfiguration,
    #[serde(default)]
    pub leverage: PaperLeverageConfiguration,
    #[serde(default)]
    pub slippage_ticks: f64,
    #[serde(default = "truth")]
    pub show_positions_on_chart: bool,
    #[serde(default = "truth")]
    pub show_orders_on_chart: bool,
    #[serde(default = "truth")]
    pub show_executions_on_chart: bool,
    #[serde(default)]
    pub instant_order_placement: bool,
}

fn truth() -> bool {
    true
}

impl Default for PaperAccountSettings {
    fn default() -> Self {
        Self {
            commission: PaperCommissionConfiguration::None,
            leverage: PaperLeverageConfiguration::default(),
            slippage_ticks: 0.0,
            show_positions_on_chart: true,
            show_orders_on_chart: true,
            show_executions_on_chart: true,
            instant_order_placement: false,
        }
    }
}

/// A tradable instrument, keyed by a source-qualified identity.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PaperInstrument {
    /// Stable identity: `"<source>:<symbol>"`.
    pub key: String,
    #[serde(default)]
    pub symbol: String,
    #[serde(default)]
    pub display_name: String,
    pub source: DataSource,
    #[serde(default)]
    pub asset_class: PaperAssetClass,
    #[serde(default)]
    pub quote_currency: PaperCurrency,
    #[serde(default)]
    pub tick_size: f64,
    #[serde(default)]
    pub minimum_quantity: f64,
    #[serde(default)]
    pub quantity_increment: f64,
    #[serde(default = "one")]
    pub contract_multiplier: f64,
    #[serde(default = "one")]
    pub point_value: f64,
    /// Epoch milliseconds; None for instruments that never expire.
    #[serde(default)]
    pub expiration: Option<i64>,
}

impl PaperInstrument {
    /// An instrument for a charted market, with per-source tick/quantity defaults.
    pub fn chart(
        symbol: impl Into<String>,
        display_name: impl Into<String>,
        source: DataSource,
    ) -> Self {
        let asset_class = if source == DataSource::Alpaca {
            PaperAssetClass::Stock
        } else if source.is_prediction_market() {
            PaperAssetClass::Prediction
        } else {
            PaperAssetClass::Crypto
        };
        let tick_size = match source {
            DataSource::Polymarket => 0.001,
            DataSource::Kalshi => 0.01,
            _ => 0.000_000_01,
        };
        let (minimum_quantity, quantity_increment) = if source == DataSource::Alpaca {
            (1.0, 1.0)
        } else {
            (0.000_000_01, 0.000_000_01)
        };
        let symbol = symbol.into();
        Self {
            key: format!("{}:{}", source.display_name(), symbol),
            symbol,
            display_name: display_name.into(),
            source,
            asset_class,
            quote_currency: PaperCurrency::Usd,
            tick_size,
            minimum_quantity,
            quantity_increment,
            contract_multiplier: 1.0,
            point_value: 1.0,
            expiration: None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PaperAccount {
    pub id: u64,
    pub name: String,
    #[serde(default)]
    pub base_currency: PaperCurrency,
    #[serde(default)]
    pub initial_balance: f64,
    #[serde(default)]
    pub cash_balance: f64,
    #[serde(default)]
    pub realized_pnl: f64,
    #[serde(default)]
    pub settings: PaperAccountSettings,
    #[serde(default)]
    pub created_at: i64,
}

impl PaperAccount {
    pub fn new(id: u64, name: impl Into<String>, created_at: i64) -> Self {
        Self {
            id,
            name: name.into(),
            base_currency: PaperCurrency::Usd,
            initial_balance: 100_000.0,
            cash_balance: 100_000.0,
            realized_pnl: 0.0,
            settings: PaperAccountSettings::default(),
            created_at,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PaperOrderRequest {
    pub account_id: u64,
    pub instrument: PaperInstrument,
    pub side: PaperOrderSide,
    #[serde(default)]
    pub kind: PaperOrderType,
    pub quantity: f64,
    #[serde(default)]
    pub limit_price: Option<f64>,
    #[serde(default)]
    pub stop_price: Option<f64>,
    #[serde(default)]
    pub time_in_force: PaperTimeInForce,
    #[serde(default)]
    pub take_profit: Option<f64>,
    #[serde(default)]
    pub stop_loss: Option<f64>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct PaperOrderChanges {
    #[serde(default)]
    pub quantity: Option<f64>,
    #[serde(default)]
    pub limit_price: Option<f64>,
    #[serde(default)]
    pub stop_price: Option<f64>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PaperOrder {
    pub id: u64,
    pub account_id: u64,
    pub instrument: PaperInstrument,
    #[serde(default)]
    pub side: PaperOrderSide,
    #[serde(default)]
    pub kind: PaperOrderType,
    pub original_quantity: f64,
    #[serde(default)]
    pub filled_quantity: f64,
    #[serde(default)]
    pub average_fill_price: Option<f64>,
    #[serde(default)]
    pub limit_price: Option<f64>,
    #[serde(default)]
    pub stop_price: Option<f64>,
    #[serde(default)]
    pub time_in_force: PaperTimeInForce,
    #[serde(default)]
    pub status: PaperOrderStatus,
    #[serde(default)]
    pub role: PaperOrderRole,
    #[serde(default)]
    pub parent_order_id: Option<u64>,
    #[serde(default)]
    pub oco_group_id: Option<u64>,
    #[serde(default)]
    pub stop_triggered: bool,
    #[serde(default)]
    pub reserved_margin: f64,
    #[serde(default)]
    pub rejection_reason: Option<String>,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
}

impl PaperOrder {
    pub fn remaining_quantity(&self) -> f64 {
        (self.original_quantity - self.filled_quantity).max(0.0)
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PaperFill {
    pub id: u64,
    pub order_id: u64,
    pub account_id: u64,
    pub instrument: PaperInstrument,
    #[serde(default)]
    pub side: PaperOrderSide,
    pub quantity: f64,
    pub price: f64,
    #[serde(default)]
    pub commission: f64,
    #[serde(default)]
    pub price_source: PaperFillPriceSource,
    /// Epoch milliseconds.
    pub timestamp: i64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PaperPosition {
    pub account_id: u64,
    pub instrument: PaperInstrument,
    #[serde(default)]
    pub signed_quantity: f64,
    #[serde(default)]
    pub average_entry_price: f64,
    #[serde(default)]
    pub realized_gross_pnl: f64,
    #[serde(default)]
    pub commissions: f64,
    #[serde(default)]
    pub opened_at: i64,
    #[serde(default)]
    pub updated_at: i64,
}

impl PaperPosition {
    pub fn side(&self) -> PaperPositionSide {
        if self.signed_quantity >= 0.0 {
            PaperPositionSide::Long
        } else {
            PaperPositionSide::Short
        }
    }

    pub fn quantity(&self) -> f64 {
        self.signed_quantity.abs()
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PaperQuote {
    pub instrument_key: String,
    #[serde(default)]
    pub bid: Option<f64>,
    #[serde(default)]
    pub ask: Option<f64>,
    #[serde(default)]
    pub last: Option<f64>,
    /// Epoch milliseconds.
    pub timestamp: i64,
    #[serde(default = "truth")]
    pub is_market_open: bool,
}

impl PaperQuote {
    /// The price a side would execute at, and which source produced it.
    pub fn executable_price(&self, side: PaperOrderSide) -> Option<(f64, PaperFillPriceSource)> {
        match side {
            PaperOrderSide::Buy => {
                if let Some(ask) = self.ask {
                    return Some((ask, PaperFillPriceSource::BidAsk));
                }
            }
            PaperOrderSide::Sell => {
                if let Some(bid) = self.bid {
                    return Some((bid, PaperFillPriceSource::BidAsk));
                }
            }
        }
        self.last
            .map(|last| (last, PaperFillPriceSource::LastPriceFallback))
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PaperOrderEventKind {
    #[default]
    Placed,
    Accepted,
    PartiallyFilled,
    Filled,
    Modified,
    Canceled,
    Rejected,
    Expired,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PaperOrderEvent {
    pub id: u64,
    pub order_id: u64,
    pub account_id: u64,
    #[serde(default)]
    pub kind: PaperOrderEventKind,
    /// Epoch milliseconds.
    pub timestamp: i64,
    #[serde(default)]
    pub message: String,
    pub order: PaperOrder,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PaperClosedTrade {
    pub id: u64,
    pub account_id: u64,
    pub instrument: PaperInstrument,
    #[serde(default)]
    pub side: PaperPositionSide,
    /// Epoch milliseconds.
    pub entry_timestamp: i64,
    #[serde(default)]
    pub exit_timestamp: i64,
    pub entry_price: f64,
    pub exit_price: f64,
    pub quantity: f64,
    pub gross_pnl: f64,
    #[serde(default)]
    pub commission: f64,
}

impl PaperClosedTrade {
    pub fn net_pnl(&self) -> f64 {
        self.gross_pnl - self.commission
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PaperJournalEntry {
    pub id: u64,
    pub account_id: u64,
    /// Epoch milliseconds.
    pub timestamp: i64,
    pub message: String,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct PaperAccountMetrics {
    #[serde(default)]
    pub balance: f64,
    #[serde(default)]
    pub equity: f64,
    #[serde(default)]
    pub realized_pnl: f64,
    #[serde(default)]
    pub unrealized_pnl: f64,
    #[serde(default)]
    pub position_margin: f64,
    #[serde(default)]
    pub orders_margin: f64,
    #[serde(default)]
    pub available_funds: f64,
    #[serde(default)]
    pub margin_buffer: f64,
}

/// The persisted paper-trading store contents, one document per collection.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct PaperTradingSnapshot {
    #[serde(default)]
    pub accounts: Vec<PaperAccount>,
    #[serde(default)]
    pub selected_account_id: Option<u64>,
    #[serde(default)]
    pub orders: Vec<PaperOrder>,
    #[serde(default)]
    pub fills: Vec<PaperFill>,
    #[serde(default)]
    pub positions: Vec<PaperPosition>,
    #[serde(default)]
    pub order_events: Vec<PaperOrderEvent>,
    #[serde(default)]
    pub closed_trades: Vec<PaperClosedTrade>,
    #[serde(default)]
    pub journal: Vec<PaperJournalEntry>,
    #[serde(default)]
    pub quotes: HashMap<String, PaperQuote>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PaperTradingError {
    AccountNotFound,
    OrderNotFound,
    InvalidTransition,
    InvalidQuantity(String),
    InvalidPrice(String),
    StaleMarketData,
    MarketClosed,
    NoMarketData,
    UnsupportedCurrencyConversion {
        from: PaperCurrency,
        to: PaperCurrency,
    },
    InsufficientFunds {
        required: f64,
        available: f64,
    },
    UnsupportedSymbol,
}

impl PaperTradingError {
    pub fn message(&self) -> String {
        match self {
            Self::AccountNotFound => "Paper account not found.".to_owned(),
            Self::OrderNotFound => "Paper order not found.".to_owned(),
            Self::InvalidTransition => "The order can no longer be changed.".to_owned(),
            Self::InvalidQuantity(message) | Self::InvalidPrice(message) => message.clone(),
            Self::StaleMarketData => {
                "Order cannot be simulated reliably because market data is stale.".to_owned()
            }
            Self::MarketClosed => {
                "The market is closed; the paper order remains pending.".to_owned()
            }
            Self::NoMarketData => "No executable market price is available.".to_owned(),
            Self::UnsupportedCurrencyConversion { from, to } => {
                format!("FX conversion from {:?} to {:?} is unavailable.", from, to)
            }
            Self::InsufficientFunds {
                required,
                available,
            } => format!(
                "Order rejected: insufficient available funds. Required margin: {required:.2}; available funds: {available:.2}."
            ),
            Self::UnsupportedSymbol => "This symbol is not supported by Paper Trading.".to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instrument(source: DataSource) -> PaperInstrument {
        PaperInstrument::chart("BTCUSDT", "Bitcoin", source)
    }

    #[test]
    fn instrument_asset_class_and_defaults() {
        let stock = instrument(DataSource::Alpaca);
        assert_eq!(stock.asset_class, PaperAssetClass::Stock);
        assert_eq!(stock.tick_size, 0.000_000_01);
        assert_eq!(stock.minimum_quantity, 1.0);

        let crypto = instrument(DataSource::Binance);
        assert_eq!(crypto.asset_class, PaperAssetClass::Crypto);

        let prediction = instrument(DataSource::Polymarket);
        assert_eq!(prediction.asset_class, PaperAssetClass::Prediction);
        assert_eq!(prediction.tick_size, 0.001);
    }

    #[test]
    fn order_status_terminal_and_working() {
        assert!(PaperOrderStatus::Filled.is_terminal());
        assert!(PaperOrderStatus::Canceled.is_terminal());
        assert!(!PaperOrderStatus::Working.is_terminal());
        assert!(PaperOrderStatus::Working.is_working());
        assert!(PaperOrderStatus::PartiallyFilled.is_working());
        assert!(!PaperOrderStatus::Filled.is_working());
    }

    #[test]
    fn order_remaining_quantity() {
        let order = PaperOrder {
            id: 1,
            account_id: 1,
            instrument: instrument(DataSource::Binance),
            side: PaperOrderSide::Buy,
            kind: PaperOrderType::Market,
            original_quantity: 10.0,
            filled_quantity: 4.0,
            average_fill_price: None,
            limit_price: None,
            stop_price: None,
            time_in_force: PaperTimeInForce::GoodTilCanceled,
            status: PaperOrderStatus::PartiallyFilled,
            role: PaperOrderRole::Entry,
            parent_order_id: None,
            oco_group_id: None,
            stop_triggered: false,
            reserved_margin: 0.0,
            rejection_reason: None,
            created_at: 0,
            updated_at: 0,
        };
        assert_eq!(order.remaining_quantity(), 6.0);
    }

    #[test]
    fn position_side_and_quantity() {
        let long = PaperPosition {
            account_id: 1,
            instrument: instrument(DataSource::Binance),
            signed_quantity: 2.0,
            average_entry_price: 100.0,
            realized_gross_pnl: 0.0,
            commissions: 0.0,
            opened_at: 0,
            updated_at: 0,
        };
        assert_eq!(long.side(), PaperPositionSide::Long);
        assert_eq!(long.quantity(), 2.0);

        let mut short = long;
        short.signed_quantity = -2.0;
        assert_eq!(short.side(), PaperPositionSide::Short);
        assert_eq!(short.quantity(), 2.0);
    }

    #[test]
    fn quote_executable_price_prefers_bid_ask() {
        let quote = PaperQuote {
            instrument_key: "binance:btc".into(),
            bid: Some(99.0),
            ask: Some(101.0),
            last: Some(100.0),
            timestamp: 0,
            is_market_open: true,
        };
        assert_eq!(
            quote.executable_price(PaperOrderSide::Buy),
            Some((101.0, PaperFillPriceSource::BidAsk))
        );
        assert_eq!(
            quote.executable_price(PaperOrderSide::Sell),
            Some((99.0, PaperFillPriceSource::BidAsk))
        );
        let last_only = PaperQuote {
            bid: None,
            ask: None,
            ..quote.clone()
        };
        assert_eq!(
            last_only.executable_price(PaperOrderSide::Buy),
            Some((100.0, PaperFillPriceSource::LastPriceFallback))
        );
        assert_eq!(
            PaperQuote {
                bid: None,
                ask: None,
                last: None,
                ..quote.clone()
            }
            .executable_price(PaperOrderSide::Buy),
            None
        );
    }

    #[test]
    fn leverage_and_commission() {
        let leverage = PaperLeverageConfiguration::default();
        assert_eq!(leverage.leverage(PaperAssetClass::Stock), 1.0);
        assert_eq!(
            PaperCommissionConfiguration::Percentage { value: 0.25 }.label(),
            "0.25% of value"
        );
        assert_eq!(PaperCommissionConfiguration::None.label(), "None");
    }

    #[test]
    fn snapshot_decodes_old_rows() {
        let snapshot: PaperTradingSnapshot = serde_json::from_str("{}").unwrap();
        assert!(snapshot.accounts.is_empty());
        assert_eq!(snapshot.selected_account_id, None);

        let commission: PaperCommissionConfiguration =
            serde_json::from_str(r#"{"kind":"percentage","value":0.5}"#).unwrap();
        assert_eq!(
            commission,
            PaperCommissionConfiguration::Percentage { value: 0.5 }
        );
    }

    #[test]
    fn account_new_defaults() {
        let account = PaperAccount::new(7, "Test", 1_000);
        assert_eq!(account.id, 7);
        assert_eq!(account.initial_balance, 100_000.0);
        assert_eq!(account.cash_balance, 100_000.0);
        assert!(account.settings.show_positions_on_chart);
    }
}
