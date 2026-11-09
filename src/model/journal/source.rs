//! The boundary a trading source implements. Synchronous and local-first: the paper source reads
//! a snapshot in memory; a broker or import adapter would do its network or file work before it
//! builds a batch, and nothing here needs an async runtime.
//!
//! A source only *reports* what happened. It has no method that places, changes or cancels an
//! order, so a read-only broker connection never needs trading permission and no journal action
//! can reach a market.

use super::model::{Execution, JournalAccount, SourceId, TradingSourceKind};

/// What a source can tell the journal. The UI and the analytics read this to show "unavailable"
/// where a figure cannot exist, rather than inventing it.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SourceCapabilities {
    pub historical_executions: bool,
    pub live_updates: bool,
    pub orders: bool,
    pub positions: bool,
    pub fees: bool,
    pub funding_payments: bool,
    pub account_balances: bool,
    pub account_currency: bool,
    pub contract_specifications: bool,
    pub broker_trade_ids: bool,
    pub historical_prices: bool,
}

/// Where a source left off. Opaque to the journal; a source decides what it means.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Cursor(pub String);

#[derive(Clone, Debug, PartialEq)]
pub enum SourceError {
    /// The account is not one this source knows.
    UnknownAccount(String),
    /// The source could not be read (a closed connection, an unreadable file).
    Unavailable(String),
}

impl std::fmt::Display for SourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownAccount(id) => write!(f, "unknown account {id}"),
            Self::Unavailable(reason) => write!(f, "source unavailable: {reason}"),
        }
    }
}

impl std::error::Error for SourceError {}

/// One pull from a source.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExecutionBatch {
    pub executions: Vec<Execution>,
    /// Source ids of executions the source has cancelled or busted since the cursor.
    pub voided: Vec<String>,
    /// Where to continue; `None` when the source has nothing further to give.
    pub next: Option<Cursor>,
    /// Fills the source could not turn into an execution, with the reason. Reported, never
    /// dropped silently.
    pub rejected: Vec<RejectedExecution>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RejectedExecution {
    pub source_exec_id: String,
    pub reason: String,
}

pub trait TradingActivitySource {
    fn source_id(&self) -> SourceId;

    fn kind(&self) -> TradingSourceKind;

    fn capabilities(&self) -> SourceCapabilities;

    /// The accounts the source has, as the journal will store them.
    fn accounts(&self) -> Vec<JournalAccount>;

    /// Executions of one account (by `JournalAccount::uid`) after `cursor`, oldest first.
    fn executions(
        &self,
        account_uid: &str,
        cursor: Option<&Cursor>,
    ) -> Result<ExecutionBatch, SourceError>;
}
