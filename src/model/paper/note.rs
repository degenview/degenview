//! What a paper account's journal and order history say, as data.
//!
//! The engine writes each line of the journal when something happens; the line stored with it
//! (`message`) is English, for the exports and MCP, and cannot follow the language chosen later.
//! A note records what happened, and the table writes the sentence when it is drawn, in the
//! language in use. A line written before notes existed has none, and shows its stored sentence.

use serde::{Deserialize, Serialize};

use super::{
    PaperCurrency, PaperFillPriceSource, PaperOrderRole, PaperOrderSide, PaperOrderType,
    PaperTradingError,
};
use crate::i18n::{tr, tr_c};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "note", rename_all = "snake_case")]
pub enum PaperNote {
    AccountCreated {
        balance: f64,
        currency: PaperCurrency,
    },
    AccountReset {
        balance: f64,
        currency: PaperCurrency,
    },
    OrderSubmitted {
        side: PaperOrderSide,
        quantity: f64,
        symbol: String,
        kind: PaperOrderType,
    },
    OrderAccepted,
    OrderCanceled,
    OrderModified,
    StopTriggered,
    Filled {
        quantity: f64,
        symbol: String,
        price: f64,
        source: PaperFillPriceSource,
        commission: f64,
    },
    RealizedPnl {
        amount: f64,
    },
    ProtectionActivated {
        role: PaperOrderRole,
    },
    OcoCounterpartCanceled,
    ProtectionClosed,
    ProtectionReversed,
    ProtectionResized {
        size: f64,
    },
    Rejected {
        error: PaperTradingError,
    },
}

impl PaperNote {
    /// The sentence, in the language in use.
    pub fn text(&self) -> String {
        match self {
            Self::AccountCreated { balance, currency } => tr!(
                "PAPER account created with {balance} {currency}",
                balance = balance,
                currency = format!("{currency:?}")
            ),
            Self::AccountReset { balance, currency } => tr!(
                "PAPER account reset to {balance} {currency}",
                balance = balance,
                currency = format!("{currency:?}")
            ),
            Self::OrderSubmitted {
                side,
                quantity,
                symbol,
                kind,
            } => tr!(
                "{side} {quantity} {symbol} {kind} order submitted",
                side = match side {
                    PaperOrderSide::Buy => tr_c!("order side in a sentence", "BUY"),
                    PaperOrderSide::Sell => tr_c!("order side in a sentence", "SELL"),
                },
                quantity = quantity,
                symbol = symbol,
                kind = match kind {
                    PaperOrderType::Market => tr_c!("order type in a sentence", "Market"),
                    PaperOrderType::Limit => tr_c!("order type in a sentence", "Limit"),
                    PaperOrderType::Stop => tr_c!("order type in a sentence", "Stop"),
                    PaperOrderType::StopLimit => tr_c!("order type in a sentence", "StopLimit"),
                }
            ),
            Self::OrderAccepted => tr!("PAPER order accepted").to_owned(),
            Self::OrderCanceled => tr!("PAPER order canceled").to_owned(),
            Self::OrderModified => tr!("PAPER order modified").to_owned(),
            Self::StopTriggered => tr!("Stop triggered; PAPER stop-limit activated").to_owned(),
            Self::Filled {
                quantity,
                symbol,
                price,
                source,
                commission,
            } => tr!(
                "Filled {quantity} {symbol} @ {price} [{source}]; commission {commission}",
                quantity = quantity,
                symbol = symbol,
                price = price,
                source = match source {
                    PaperFillPriceSource::BidAsk => tr_c!("price a fill used", "BidAsk"),
                    PaperFillPriceSource::LastPriceFallback => {
                        tr_c!("price a fill used", "LastPriceFallback")
                    }
                    PaperFillPriceSource::BarFallback => {
                        tr_c!("price a fill used", "BarFallback")
                    }
                },
                commission = commission
            ),
            Self::RealizedPnl { amount } => tr!(
                "Realized gross P&L {amount} before commission",
                amount = amount
            ),
            Self::ProtectionActivated { role } => tr!(
                "PAPER {role} protection activated",
                role = match role {
                    PaperOrderRole::Entry => tr_c!("order role in a sentence", "Entry"),
                    PaperOrderRole::TakeProfit => tr_c!("order role in a sentence", "TakeProfit"),
                    PaperOrderRole::StopLoss => tr_c!("order role in a sentence", "StopLoss"),
                }
            ),
            Self::OcoCounterpartCanceled => tr!("PAPER OCO counterpart canceled").to_owned(),
            Self::ProtectionClosed => {
                tr!("PAPER protection canceled; the position is closed").to_owned()
            }
            Self::ProtectionReversed => {
                tr!("PAPER protection canceled; the position reversed").to_owned()
            }
            Self::ProtectionResized { size } => tr!(
                "PAPER protection resized to the remaining {size}",
                size = size
            ),
            Self::Rejected { error } => error.text(),
        }
    }
}
