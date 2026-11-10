//! The paper-trading source. It reads a [`PaperTradingSnapshot`] and nothing else: no host, no
//! engine, no way to place or change an order, so the journal cannot move a paper balance.
//!
//! Identity. The engine numbers fills with a per-Mac counter that is reused after a reset and
//! tagged when it syncs, so its ids cannot name a fill across devices or lifetimes. A paper
//! account is therefore `paper:<created_at>` (a reset gives the account a new `created_at`, so
//! it is a new generation) and a fill is `<account>:<timestamp>:<k>` where `k` counts the fills
//! of that account with the same timestamp, in ledger order. Both are the same on every Mac that
//! holds the account. The engine's own fill id stays in `source_exec_id` for audit.

use std::collections::HashMap;

use rust_decimal::Decimal;

use crate::model::InstrumentId;
use crate::model::paper::{
    PaperAccount, PaperAssetClass, PaperCurrency, PaperFill, PaperOrder, PaperOrderEventKind,
    PaperOrderRole, PaperOrderSide, PaperTradingSnapshot,
};
use degenview_journal::accounting::from_f64;
use degenview_journal::model::{
    AssetClass, CostBasis, Execution, ExecutionStatus, InstrumentRef, JournalAccount, Side,
    SourceId, TradingSourceKind,
};
use degenview_journal::source::{
    Cursor, ExecutionBatch, RejectedExecution, SourceCapabilities, SourceError,
    TradingActivitySource,
};

pub struct PaperSource<'a> {
    snapshot: &'a PaperTradingSnapshot,
    /// Journal uid per paper account id.
    uids: HashMap<u64, String>,
}

impl<'a> PaperSource<'a> {
    pub fn new(snapshot: &'a PaperTradingSnapshot) -> Self {
        // Two accounts created in the same millisecond cannot be told apart by time alone; the
        // later one by id gets a suffix, the same on every device.
        let mut ordered: Vec<&PaperAccount> = snapshot.accounts.iter().collect();
        ordered.sort_by_key(|account| (account.created_at, account.id));
        let mut seen: HashMap<i64, usize> = HashMap::new();
        let mut uids = HashMap::new();
        for account in ordered {
            let count = seen.entry(account.created_at).or_insert(0);
            let uid = if *count == 0 {
                format!("paper:{}", account.created_at)
            } else {
                format!("paper:{}-{}", account.created_at, count)
            };
            *count += 1;
            uids.insert(account.id, uid);
        }
        Self { snapshot, uids }
    }

    /// The journal uid of a paper account.
    #[allow(dead_code)] // used by the Journal tab, which follows
    pub fn account_uid(&self, account_id: u64) -> Option<&str> {
        self.uids.get(&account_id).map(String::as_str)
    }

    fn account_for(&self, uid: &str) -> Option<&PaperAccount> {
        self.snapshot
            .accounts
            .iter()
            .find(|account| self.uids.get(&account.id).is_some_and(|own| own == uid))
    }

    /// The stop attached to the entry order that produced `fill`, as first placed: the earliest
    /// event of the protective child records its price before anyone moved it.
    fn planned_stop(&self, fill: &PaperFill) -> Option<Decimal> {
        let entry = self
            .snapshot
            .orders
            .iter()
            .find(|order| order.id == fill.order_id)?;
        if entry.role != PaperOrderRole::Entry {
            return None;
        }
        let child: &PaperOrder = self.snapshot.orders.iter().find(|order| {
            order.parent_order_id == Some(entry.id) && order.role == PaperOrderRole::StopLoss
        })?;
        let placed = self
            .snapshot
            .order_events
            .iter()
            .filter(|event| {
                event.order_id == child.id && event.kind == PaperOrderEventKind::Accepted
            })
            .map(|event| &event.order)
            .next();
        placed.unwrap_or(child).stop_price.and_then(from_f64)
    }
}

fn currency_code(currency: PaperCurrency) -> String {
    serde_json::to_value(currency)
        .ok()
        .and_then(|value| value.as_str().map(str::to_uppercase))
        .unwrap_or_default()
}

fn asset_class(class: PaperAssetClass) -> AssetClass {
    match class {
        PaperAssetClass::Stock => AssetClass::Stock,
        PaperAssetClass::Crypto => AssetClass::Crypto,
        PaperAssetClass::Forex => AssetClass::Forex,
        PaperAssetClass::Futures => AssetClass::Futures,
        PaperAssetClass::Prediction => AssetClass::Prediction,
    }
}

impl TradingActivitySource for PaperSource<'_> {
    fn source_id(&self) -> SourceId {
        SourceId::paper()
    }

    fn kind(&self) -> TradingSourceKind {
        TradingSourceKind::Paper
    }

    fn capabilities(&self) -> SourceCapabilities {
        SourceCapabilities {
            historical_executions: true,
            live_updates: true,
            orders: true,
            positions: true,
            fees: true,
            funding_payments: false,
            account_balances: true,
            account_currency: true,
            contract_specifications: true,
            broker_trade_ids: false,
            historical_prices: false,
        }
    }

    fn accounts(&self) -> Vec<JournalAccount> {
        self.snapshot
            .accounts
            .iter()
            .filter_map(|account| {
                Some(JournalAccount {
                    uid: self.uids.get(&account.id)?.clone(),
                    source: SourceId::paper(),
                    kind: TradingSourceKind::Paper,
                    external_id: account.id.to_string(),
                    name: account.name.clone(),
                    currency: currency_code(account.base_currency),
                    initial_balance: from_f64(account.initial_balance),
                    // The engine keeps one running average entry price.
                    cost_basis: CostBasis::AverageCost,
                    archived: false,
                    created_at_ms: account.created_at,
                })
            })
            .collect()
    }

    fn executions(
        &self,
        account_uid: &str,
        cursor: Option<&Cursor>,
    ) -> Result<ExecutionBatch, SourceError> {
        let account = self
            .account_for(account_uid)
            .ok_or_else(|| SourceError::UnknownAccount(account_uid.to_owned()))?;
        let skip = cursor
            .and_then(|cursor| cursor.0.parse::<usize>().ok())
            .unwrap_or(0);
        let currency = currency_code(account.base_currency);
        let fills: Vec<&PaperFill> = self
            .snapshot
            .fills
            .iter()
            .filter(|fill| fill.account_id == account.id)
            .collect();

        // `k`: how many fills of this account before this one share its timestamp.
        let mut same_time: HashMap<i64, usize> = HashMap::new();
        let mut batch = ExecutionBatch {
            next: Some(Cursor(fills.len().to_string())),
            ..ExecutionBatch::default()
        };
        for (index, fill) in fills.iter().enumerate() {
            let k = same_time.entry(fill.timestamp).or_insert(0);
            let ordinal = *k;
            *k += 1;
            if index < skip {
                continue;
            }
            match self.execution(account_uid, &currency, fill, ordinal) {
                Ok(execution) => batch.executions.push(execution),
                Err(reason) => batch.rejected.push(RejectedExecution {
                    source_exec_id: fill.id.to_string(),
                    reason,
                }),
            }
        }
        Ok(batch)
    }
}

impl PaperSource<'_> {
    fn execution(
        &self,
        account_uid: &str,
        currency: &str,
        fill: &PaperFill,
        ordinal: usize,
    ) -> Result<Execution, String> {
        let quantity = from_f64(fill.quantity)
            .ok_or_else(|| format!("quantity {} is not a number", fill.quantity))?;
        let price =
            from_f64(fill.price).ok_or_else(|| format!("price {} is not a number", fill.price))?;
        let fee = from_f64(fill.commission)
            .ok_or_else(|| format!("commission {} is not a number", fill.commission))?;
        let multiplier = from_f64(fill.instrument.point_value).ok_or_else(|| {
            format!(
                "point value {} is not a number",
                fill.instrument.point_value
            )
        })?;
        let instrument = &fill.instrument;
        Ok(Execution {
            uid: format!("{account_uid}:{}:{ordinal}", fill.timestamp),
            source: SourceId::paper(),
            account_uid: account_uid.to_owned(),
            source_exec_id: fill.id.to_string(),
            order_ref: Some(fill.order_id.to_string()),
            instrument: InstrumentRef {
                key: instrument.key.clone(),
                market_key: InstrumentId::new(instrument.source, instrument.symbol.clone()).key(),
                symbol: instrument.symbol.clone(),
                display_name: instrument.display_name.clone(),
                provider: Some(instrument.source.display_name().to_owned()),
                asset_class: asset_class(instrument.asset_class),
                quote_currency: Some(currency_code(instrument.quote_currency)),
            },
            side: match fill.side {
                PaperOrderSide::Buy => Side::Buy,
                PaperOrderSide::Sell => Side::Sell,
            },
            quantity,
            price,
            ts_ms: fill.timestamp,
            source_tz: None,
            fee: Some(fee),
            fee_currency: Some(currency.to_owned()),
            liquidity: None,
            multiplier,
            settlement_currency: Some(currency.to_owned()),
            planned_stop: self.planned_stop(fill),
            status: ExecutionStatus::Active,
            meta: serde_json::json!({
                "fill_id": fill.id,
                "price_source": serde_json::to_value(fill.price_source).unwrap_or_default(),
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DataSource;
    use crate::model::paper::{
        PaperAccountSettings, PaperCommissionConfiguration, PaperInstrument, PaperOrderRequest,
        PaperOrderType, PaperQuote, PaperTimeInForce,
    };
    use crate::model::paper_engine::PaperTradingEngine;
    use degenview_journal::reconstruct::reconstruct;
    use degenview_journal::testing::d;

    fn now() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_millis() as i64)
    }

    fn instrument() -> PaperInstrument {
        PaperInstrument::chart("BTCUSDT", "BTCUSDT", DataSource::Binance)
    }

    fn quote(last: f64) -> PaperQuote {
        PaperQuote {
            instrument_key: instrument().key,
            bid: Some(last - 0.5),
            ask: Some(last + 0.5),
            last: Some(last),
            timestamp: now(),
            is_market_open: true,
        }
    }

    fn request(account_id: u64, side: PaperOrderSide, quantity: f64) -> PaperOrderRequest {
        PaperOrderRequest {
            account_id,
            instrument: instrument(),
            side,
            kind: PaperOrderType::Market,
            quantity,
            limit_price: None,
            stop_price: None,
            time_in_force: PaperTimeInForce::GoodTilCanceled,
            take_profit: None,
            stop_loss: None,
        }
    }

    fn engine(commission: PaperCommissionConfiguration) -> (PaperTradingEngine, u64) {
        let mut engine = PaperTradingEngine::new(PaperTradingSnapshot::default());
        let settings = PaperAccountSettings {
            commission,
            ..PaperAccountSettings::default()
        };
        let id = engine
            .create_account("Main", PaperCurrency::Usd, 100_000.0, settings)
            .unwrap();
        (engine, id)
    }

    #[test]
    fn an_account_and_its_fills_become_executions() {
        let (mut engine, id) = engine(PaperCommissionConfiguration::FixedPerOrder { value: 1.5 });
        engine.process(quote(100.0)).unwrap();
        engine
            .submit(request(id, PaperOrderSide::Buy, 2.0))
            .unwrap();
        engine.process(quote(110.0)).unwrap();
        engine
            .submit(request(id, PaperOrderSide::Sell, 2.0))
            .unwrap();

        let snapshot = engine.snapshot();
        let source = PaperSource::new(snapshot);
        let accounts = source.accounts();
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].currency, "USD");
        assert_eq!(accounts[0].initial_balance, Some(d("100000")));
        assert!(accounts[0].uid.starts_with("paper:"));

        let batch = source.executions(&accounts[0].uid, None).unwrap();
        assert!(batch.rejected.is_empty());
        assert_eq!(batch.executions.len(), 2);
        let buy = &batch.executions[0];
        assert_eq!(buy.side, Side::Buy);
        assert_eq!(buy.quantity, d("2"));
        assert_eq!(buy.price, d("100.5")); // the ask of a 100 quote
        assert_eq!(buy.fee, Some(d("1.5")));
        assert_eq!(buy.settlement_currency.as_deref(), Some("USD"));
        assert_eq!(buy.instrument.provider.as_deref(), Some("Binance"));
        assert_eq!(buy.instrument.market_key, "BINANCE:BTCUSDT");
        assert!(buy.uid.starts_with(&accounts[0].uid));
        assert_ne!(buy.uid, batch.executions[1].uid);
        assert_eq!(batch.next, Some(Cursor("2".to_owned())));
    }

    #[test]
    fn a_cursor_returns_only_what_came_after() {
        let (mut engine, id) = engine(PaperCommissionConfiguration::None);
        engine.process(quote(100.0)).unwrap();
        engine
            .submit(request(id, PaperOrderSide::Buy, 1.0))
            .unwrap();
        engine
            .submit(request(id, PaperOrderSide::Buy, 1.0))
            .unwrap();
        let snapshot = engine.snapshot();
        let source = PaperSource::new(snapshot);
        let uid = source.accounts()[0].uid.clone();
        let all = source.executions(&uid, None).unwrap();
        let after = source
            .executions(&uid, Some(&Cursor("1".to_owned())))
            .unwrap();
        assert_eq!(after.executions, all.executions[1..]);
    }

    #[test]
    fn fills_in_the_same_millisecond_keep_distinct_identities() {
        let (mut engine, id) = engine(PaperCommissionConfiguration::None);
        engine.process(quote(100.0)).unwrap();
        for _ in 0..4 {
            engine
                .submit(request(id, PaperOrderSide::Buy, 1.0))
                .unwrap();
        }
        let source = PaperSource::new(engine.snapshot());
        let uid = source.accounts()[0].uid.clone();
        let batch = source.executions(&uid, None).unwrap();
        let mut uids: Vec<_> = batch.executions.iter().map(|e| e.uid.clone()).collect();
        uids.sort();
        uids.dedup();
        assert_eq!(uids.len(), 4);
    }

    #[test]
    fn an_unknown_account_is_an_error() {
        let snapshot = PaperTradingSnapshot::default();
        let source = PaperSource::new(&snapshot);
        assert_eq!(
            source.executions("paper:1", None),
            Err(SourceError::UnknownAccount("paper:1".to_owned()))
        );
    }

    #[test]
    fn a_bracket_stop_is_the_planned_stop_even_if_it_was_moved() {
        let (mut engine, id) = engine(PaperCommissionConfiguration::None);
        engine.process(quote(100.0)).unwrap();
        let mut entry = request(id, PaperOrderSide::Buy, 1.0);
        entry.stop_loss = Some(95.0);
        entry.take_profit = Some(120.0);
        engine.submit(entry).unwrap();
        let stop = engine
            .snapshot()
            .orders
            .iter()
            .find(|order| order.role == PaperOrderRole::StopLoss)
            .unwrap()
            .id;
        engine
            .modify(
                stop,
                crate::model::paper::PaperOrderChanges {
                    stop_price: Some(99.0),
                    ..Default::default()
                },
            )
            .unwrap();

        let source = PaperSource::new(engine.snapshot());
        let uid = source.accounts()[0].uid.clone();
        let batch = source.executions(&uid, None).unwrap();
        assert_eq!(batch.executions[0].planned_stop, Some(d("95")));
    }

    #[test]
    fn an_order_without_a_stop_has_no_planned_stop() {
        let (mut engine, id) = engine(PaperCommissionConfiguration::None);
        engine.process(quote(100.0)).unwrap();
        engine
            .submit(request(id, PaperOrderSide::Buy, 1.0))
            .unwrap();
        let source = PaperSource::new(engine.snapshot());
        let uid = source.accounts()[0].uid.clone();
        assert_eq!(
            source.executions(&uid, None).unwrap().executions[0].planned_stop,
            None
        );
    }

    #[test]
    fn two_accounts_made_in_one_millisecond_still_have_their_own_uids() {
        let mut snapshot = PaperTradingSnapshot::default();
        snapshot.accounts.push(PaperAccount::new(2, "B", 5_000));
        snapshot.accounts.push(PaperAccount::new(1, "A", 5_000));
        let source = PaperSource::new(&snapshot);
        assert_eq!(source.account_uid(1), Some("paper:5000"));
        assert_eq!(source.account_uid(2), Some("paper:5000-1"));
    }

    /// The journal's average-cost accounting reproduces the engine's own closed trades.
    #[test]
    fn journal_trades_match_the_engines_closed_trades() {
        let (mut engine, id) = engine(PaperCommissionConfiguration::Percentage { value: 0.1 });
        let steps = [
            (100.0, PaperOrderSide::Buy, 2.0),
            (104.0, PaperOrderSide::Buy, 1.0),
            (110.0, PaperOrderSide::Sell, 1.5),
            (95.0, PaperOrderSide::Sell, 1.5),
            (96.0, PaperOrderSide::Sell, 3.0),
            (90.0, PaperOrderSide::Buy, 1.0),
            (92.0, PaperOrderSide::Buy, 2.0),
        ];
        for (price, side, quantity) in steps {
            engine.process(quote(price)).unwrap();
            engine.submit(request(id, side, quantity)).unwrap();
        }
        let snapshot = engine.snapshot();
        assert!(snapshot.positions.is_empty(), "the sequence ends flat");

        let source = PaperSource::new(snapshot);
        let uid = source.accounts()[0].uid.clone();
        let executions = source.executions(&uid, None).unwrap().executions;
        let trades = reconstruct(&executions, |_| CostBasis::AverageCost);

        let engine_net: f64 = snapshot.closed_trades.iter().map(|t| t.net_pnl()).sum();
        let journal_net: Decimal = trades.iter().map(|trade| trade.net_pnl()).sum();
        let journal_net: f64 = journal_net.to_string().parse().unwrap();
        assert!(
            (engine_net - journal_net).abs() < 1e-6,
            "engine {engine_net} vs journal {journal_net}"
        );
        // The account's own books agree too.
        let account = &snapshot.accounts[0];
        assert!((account.realized_pnl - journal_net).abs() < 1e-6);
        assert!(trades.iter().all(|trade| !trade.is_open()));
    }
}
