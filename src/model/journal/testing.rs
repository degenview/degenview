//! Fixture builders shared by the journal's tests.

use rust_decimal::Decimal;
use std::str::FromStr;

use super::model::{AssetClass, Execution, ExecutionStatus, InstrumentRef, Side, SourceId};

pub fn d(text: &str) -> Decimal {
    Decimal::from_str(text).unwrap()
}

pub fn instrument(symbol: &str) -> InstrumentRef {
    InstrumentRef {
        key: format!("Test:{symbol}"),
        market_key: format!("TEST:{symbol}"),
        symbol: symbol.to_owned(),
        display_name: symbol.to_owned(),
        provider: Some("Test".to_owned()),
        asset_class: AssetClass::Crypto,
        quote_currency: Some("USD".to_owned()),
    }
}

/// A buy or sell of `quantity` at `price` at `ts_ms`, with no fee, in account `acct`.
pub fn exec(id: &str, side: Side, quantity: &str, price: &str, ts_ms: i64) -> Execution {
    Execution {
        uid: format!("acct|{id}"),
        source: SourceId("test".to_owned()),
        account_uid: "acct".to_owned(),
        source_exec_id: id.to_owned(),
        order_ref: None,
        instrument: instrument("BTC"),
        side,
        quantity: d(quantity),
        price: d(price),
        ts_ms,
        source_tz: None,
        fee: Some(Decimal::ZERO),
        fee_currency: Some("USD".to_owned()),
        liquidity: None,
        multiplier: Decimal::ONE,
        settlement_currency: Some("USD".to_owned()),
        planned_stop: None,
        status: ExecutionStatus::Active,
        meta: serde_json::Value::Null,
    }
}

impl Execution {
    pub fn fee(mut self, fee: &str) -> Self {
        self.fee = Some(d(fee));
        self
    }

    pub fn no_fee_figure(mut self) -> Self {
        self.fee = None;
        self
    }

    pub fn in_account(mut self, account: &str) -> Self {
        self.account_uid = account.to_owned();
        self.uid = format!("{account}|{}", self.source_exec_id);
        self
    }

    pub fn of(mut self, symbol: &str) -> Self {
        self.instrument = instrument(symbol);
        self
    }

    pub fn times(mut self, multiplier: &str) -> Self {
        self.multiplier = d(multiplier);
        self
    }

    pub fn stop(mut self, stop: &str) -> Self {
        self.planned_stop = Some(d(stop));
        self
    }
}
