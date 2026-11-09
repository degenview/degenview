//! Canonical journal records. A source-independent execution is the only input; trades are
//! derived from executions and never stored as the authority.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// Who produced an execution. A short stable slug: `"paper"`, a broker id, an import profile.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct SourceId(pub String);

impl SourceId {
    pub const PAPER: &'static str = "paper";

    pub fn paper() -> Self {
        Self(Self::PAPER.to_owned())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TradingSourceKind {
    Paper,
    Broker,
    Imported,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Buy,
    Sell,
}

impl Side {
    /// +1 for a buy, -1 for a sell: the sign this side adds to a position.
    pub fn sign(self) -> Decimal {
        match self {
            Self::Buy => Decimal::ONE,
            Self::Sell => Decimal::NEGATIVE_ONE,
        }
    }

    pub fn opposite(self) -> Self {
        match self {
            Self::Buy => Self::Sell,
            Self::Sell => Self::Buy,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetClass {
    Stock,
    Crypto,
    Forex,
    Futures,
    Prediction,
    #[default]
    Other,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Liquidity {
    Maker,
    Taker,
}

/// An instrument as a source named it. `key` is the source's own identity (the grouping key of a
/// position, so two spellings a source treats as two positions stay two); `market_key` is the
/// normalised market identity used to find a chart and to aggregate by instrument.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct InstrumentRef {
    pub key: String,
    pub market_key: String,
    pub symbol: String,
    pub display_name: String,
    /// The market-data provider's name, when the source's instruments map to one.
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub asset_class: AssetClass,
    #[serde(default)]
    pub quote_currency: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStatus {
    #[default]
    Active,
    /// Cancelled or busted by the source. Kept for audit, ignored by reconstruction.
    Voided,
}

/// One fill, normalised. Immutable once ingested except by an explicit source correction.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Execution {
    /// Globally unique and stable across devices: the journal's identity for this fill. Reviews
    /// and chart links refer to it.
    pub uid: String,
    pub source: SourceId,
    /// The account's uid in the journal (`JournalAccount::uid`).
    pub account_uid: String,
    /// The source's own id for the fill, kept as written for audit.
    pub source_exec_id: String,
    #[serde(default)]
    pub order_ref: Option<String>,
    pub instrument: InstrumentRef,
    pub side: Side,
    /// Always positive.
    pub quantity: Decimal,
    pub price: Decimal,
    /// Epoch milliseconds, UTC.
    pub ts_ms: i64,
    /// The zone the source reported in, where it said (imports).
    #[serde(default)]
    pub source_tz: Option<String>,
    /// `None` when the source gave no fee, which is not the same as a zero fee.
    #[serde(default)]
    pub fee: Option<Decimal>,
    #[serde(default)]
    pub fee_currency: Option<String>,
    #[serde(default)]
    pub liquidity: Option<Liquidity>,
    /// Value of one price point per unit (paper `point_value`).
    pub multiplier: Decimal,
    #[serde(default)]
    pub settlement_currency: Option<String>,
    /// The protective stop that was attached to the order, when the source said so. Never
    /// inferred.
    #[serde(default)]
    pub planned_stop: Option<Decimal>,
    #[serde(default)]
    pub status: ExecutionStatus,
    /// Source-specific values kept for audit, not read by the journal's logic.
    #[serde(default)]
    pub meta: serde_json::Value,
}

/// How reductions are matched to entries when a position is built from several fills.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CostBasis {
    /// One running average entry price. What the paper engine does, so paper accounts use it.
    #[default]
    AverageCost,
    Fifo,
    Lifo,
}

/// A trading account as the journal knows it.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct JournalAccount {
    pub uid: String,
    pub source: SourceId,
    pub kind: TradingSourceKind,
    /// The source's id for the account, as written.
    pub external_id: String,
    pub name: String,
    /// ISO-style code (`"USD"`, `"USDT"`).
    pub currency: String,
    #[serde(default)]
    pub initial_balance: Option<Decimal>,
    #[serde(default)]
    pub cost_basis: CostBasis,
    /// The source no longer has this account (a paper account that was reset or deleted). Its
    /// history stays.
    #[serde(default)]
    pub archived: bool,
    pub created_at_ms: i64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Long,
    Short,
}

impl Direction {
    pub fn sign(self) -> Decimal {
        match self {
            Self::Long => Decimal::ONE,
            Self::Short => Decimal::NEGATIVE_ONE,
        }
    }

    /// The side of an execution that opens or adds to this direction.
    pub fn entry_side(self) -> Side {
        match self {
            Self::Long => Side::Buy,
            Self::Short => Side::Sell,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TradeStatus {
    Open,
    Closed,
}

/// The part of one execution that belongs to one trade. A reversal fill is split in two legs, one
/// on each trade; fees follow the quantity.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TradeLeg {
    pub exec_uid: String,
    pub ts_ms: i64,
    pub side: Side,
    pub quantity: Decimal,
    pub price: Decimal,
    /// The fee share of this part; `None` when the execution had no fee figure.
    pub fee: Option<Decimal>,
    /// True when the leg opened or added to the position, false when it reduced it.
    pub is_entry: bool,
    /// Gross P&L this leg realised (zero for entries), by the account's cost basis.
    pub realized_gross: Decimal,
    /// Position size after the leg, unsigned.
    pub position_after: Decimal,
}

/// A flat-to-flat round trip in one instrument of one account (or the open one in progress).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Trade {
    /// The uid of the execution that opened it. Unique: an execution opens at most one trade.
    pub key: String,
    pub account_uid: String,
    pub instrument: InstrumentRef,
    pub direction: Direction,
    pub status: TradeStatus,
    pub open_ts_ms: i64,
    pub close_ts_ms: Option<i64>,
    pub legs: Vec<TradeLeg>,
    /// Total quantity bought (long) or sold (short) into the trade.
    pub entry_quantity: Decimal,
    pub exit_quantity: Decimal,
    /// Size still open; zero once closed.
    pub open_quantity: Decimal,
    /// Largest size held at any moment.
    pub max_quantity: Decimal,
    pub avg_entry: Decimal,
    /// `None` until something was closed.
    pub avg_exit: Option<Decimal>,
    pub multiplier: Decimal,
    /// Gross P&L realised so far (the whole result once closed).
    pub gross_pnl: Decimal,
    /// Fees charged so far: every exit fee plus the entry fees of the quantity already closed.
    pub fees: Decimal,
    /// False when any contributing execution had no fee figure; `fees` then covers only the
    /// known ones and net P&L is not final.
    pub fees_known: bool,
    /// Entry fees still attached to the open quantity.
    pub open_fees: Decimal,
    pub currency: Option<String>,
    pub initial_stop: Option<Decimal>,
}

impl Trade {
    pub fn net_pnl(&self) -> Decimal {
        self.gross_pnl - self.fees
    }

    pub fn is_open(&self) -> bool {
        self.status == TradeStatus::Open
    }

    /// Time held, in milliseconds. `None` while open.
    pub fn duration_ms(&self) -> Option<i64> {
        self.close_ts_ms.map(|close| close - self.open_ts_ms)
    }
}
