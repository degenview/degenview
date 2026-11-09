//! Trade reconstruction: executions in, flat-to-flat trades out. Pure and deterministic.
//!
//! Rules (also in `docs/trading-journal.md`):
//!
//! 1. Executions are grouped by `(account, instrument key)` and replayed in `(timestamp, input
//!    order)` order. Voided executions are ignored.
//! 2. A trade opens with the first execution on a flat position and closes when the position is
//!    flat again. Scaling in and out stays inside one trade.
//! 3. A fill that crosses flat is split: the quantity that closes the position ends the trade, the
//!    rest opens the next one. The fee follows the quantity, the remainder going to the opening
//!    part so the two parts always add back to the fee.
//! 4. Entry fees stay attached to the open quantity and are charged pro rata as it closes, the
//!    way the paper engine does. A trade's `fees` is therefore everything charged so far.
//! 5. Gross P&L of a reduction follows the account's [`CostBasis`]. A closed trade's total is the
//!    same under every basis; which leg earned it is not.

use std::collections::{HashMap, VecDeque};

use rust_decimal::Decimal;

use super::accounting::{share, weighted_average};
use super::model::{
    CostBasis, Direction, Execution, ExecutionStatus, Side, Trade, TradeLeg, TradeStatus,
};

#[derive(Clone, Debug)]
struct Lot {
    quantity: Decimal,
    price: Decimal,
}

/// The trade being built for one position.
struct Building {
    key: String,
    account_uid: String,
    instrument: super::model::InstrumentRef,
    direction: Direction,
    open_ts_ms: i64,
    multiplier: Decimal,
    currency: Option<String>,
    initial_stop: Option<Decimal>,
    legs: Vec<TradeLeg>,
    lots: VecDeque<Lot>,
    open_quantity: Decimal,
    max_quantity: Decimal,
    fee_pool: Decimal,
    fees: Decimal,
    fees_known: bool,
}

impl Building {
    fn open(execution: &Execution) -> Self {
        Self {
            key: execution.uid.clone(),
            account_uid: execution.account_uid.clone(),
            instrument: execution.instrument.clone(),
            direction: match execution.side {
                Side::Buy => Direction::Long,
                Side::Sell => Direction::Short,
            },
            open_ts_ms: execution.ts_ms,
            multiplier: execution.multiplier,
            currency: execution.settlement_currency.clone(),
            initial_stop: execution.planned_stop,
            legs: Vec::new(),
            lots: VecDeque::new(),
            open_quantity: Decimal::ZERO,
            max_quantity: Decimal::ZERO,
            fee_pool: Decimal::ZERO,
            fees: Decimal::ZERO,
            fees_known: true,
        }
    }

    fn add(
        &mut self,
        execution: &Execution,
        quantity: Decimal,
        fee: Option<Decimal>,
        basis: CostBasis,
    ) {
        match fee {
            Some(fee) => self.fee_pool += fee,
            None => self.fees_known = false,
        }
        match basis {
            CostBasis::AverageCost => match self.lots.front_mut() {
                Some(lot) => {
                    let total = lot.quantity + quantity;
                    lot.price = (lot.price * lot.quantity + execution.price * quantity) / total;
                    lot.quantity = total;
                }
                None => self.lots.push_back(Lot {
                    quantity,
                    price: execution.price,
                }),
            },
            CostBasis::Fifo | CostBasis::Lifo => self.lots.push_back(Lot {
                quantity,
                price: execution.price,
            }),
        }
        // The pool and its quantity are tracked together so an unknown fee still attaches its
        // quantity: the known fees are spread over all of it.
        self.open_quantity += quantity;
        self.max_quantity = self.max_quantity.max(self.open_quantity);
        self.legs.push(TradeLeg {
            exec_uid: execution.uid.clone(),
            ts_ms: execution.ts_ms,
            side: execution.side,
            quantity,
            price: execution.price,
            fee,
            is_entry: true,
            realized_gross: Decimal::ZERO,
            position_after: self.open_quantity,
        });
    }

    fn reduce(
        &mut self,
        execution: &Execution,
        quantity: Decimal,
        fee: Option<Decimal>,
        basis: CostBasis,
    ) {
        let realized = self.consume_lots(quantity, execution.price, basis);
        // Entry fees leave the pool in proportion to the quantity that closes.
        let allocated = share(self.fee_pool, quantity, self.open_quantity);
        self.fee_pool -= allocated;
        match fee {
            Some(fee) => self.fees += allocated + fee,
            None => {
                self.fees_known = false;
                self.fees += allocated;
            }
        }
        self.open_quantity -= quantity;
        self.legs.push(TradeLeg {
            exec_uid: execution.uid.clone(),
            ts_ms: execution.ts_ms,
            side: execution.side,
            quantity,
            price: execution.price,
            fee,
            is_entry: false,
            realized_gross: realized,
            position_after: self.open_quantity,
        });
    }

    /// Takes `quantity` out of the lots by `basis` and returns the gross P&L realised at `price`.
    fn consume_lots(&mut self, quantity: Decimal, price: Decimal, basis: CostBasis) -> Decimal {
        let direction = self.direction.sign();
        let mut remaining = quantity;
        let mut gross = Decimal::ZERO;
        while remaining > Decimal::ZERO {
            let lot = match basis {
                CostBasis::Lifo => self.lots.back_mut(),
                CostBasis::AverageCost | CostBasis::Fifo => self.lots.front_mut(),
            };
            let Some(lot) = lot else { break };
            let taken = remaining.min(lot.quantity);
            gross += (price - lot.price) * taken * direction * self.multiplier;
            lot.quantity -= taken;
            remaining -= taken;
            if lot.quantity.is_zero() {
                match basis {
                    CostBasis::Lifo => {
                        self.lots.pop_back();
                    }
                    CostBasis::AverageCost | CostBasis::Fifo => {
                        self.lots.pop_front();
                    }
                }
            }
        }
        gross
    }

    fn finish(self, status: TradeStatus) -> Trade {
        let entries = self.legs.iter().filter(|leg| leg.is_entry);
        let exits = self.legs.iter().filter(|leg| !leg.is_entry);
        let entry_quantity: Decimal = entries.clone().map(|leg| leg.quantity).sum();
        let exit_quantity: Decimal = exits.clone().map(|leg| leg.quantity).sum();
        let avg_entry =
            weighted_average(entries.map(|leg| (leg.quantity, leg.price))).unwrap_or(Decimal::ZERO);
        let avg_exit = weighted_average(exits.map(|leg| (leg.quantity, leg.price)));
        let gross_pnl = self.legs.iter().map(|leg| leg.realized_gross).sum();
        let close_ts_ms = (status == TradeStatus::Closed)
            .then(|| self.legs.last().map(|leg| leg.ts_ms))
            .flatten();
        Trade {
            key: self.key,
            account_uid: self.account_uid,
            instrument: self.instrument,
            direction: self.direction,
            status,
            open_ts_ms: self.open_ts_ms,
            close_ts_ms,
            legs: self.legs,
            entry_quantity,
            exit_quantity,
            open_quantity: self.open_quantity,
            max_quantity: self.max_quantity,
            avg_entry,
            avg_exit,
            multiplier: self.multiplier,
            gross_pnl,
            fees: self.fees,
            fees_known: self.fees_known,
            open_fees: self.fee_pool,
            currency: self.currency,
            initial_stop: self.initial_stop,
        }
    }
}

/// Replays `executions` into trades, oldest open first. `basis_of` names each account's cost
/// basis by account uid.
pub fn reconstruct(executions: &[Execution], basis_of: impl Fn(&str) -> CostBasis) -> Vec<Trade> {
    let mut groups: HashMap<(&str, &str), Vec<(usize, &Execution)>> = HashMap::new();
    for (index, execution) in executions.iter().enumerate() {
        if execution.status != ExecutionStatus::Active || execution.quantity <= Decimal::ZERO {
            continue;
        }
        groups
            .entry((&execution.account_uid, &execution.instrument.key))
            .or_default()
            .push((index, execution));
    }

    let mut trades = Vec::new();
    for ((account_uid, _), mut group) in groups {
        group.sort_by_key(|(index, execution)| (execution.ts_ms, *index));
        let basis = basis_of(account_uid);
        let mut current: Option<Building> = None;
        for (_, execution) in group {
            let mut remaining = execution.quantity;
            let mut fee = execution.fee;
            while remaining > Decimal::ZERO {
                let Some(building) = current.as_mut() else {
                    let mut opened = Building::open(execution);
                    opened.add(execution, remaining, fee, basis);
                    current = Some(opened);
                    break;
                };
                if execution.side == building.direction.entry_side() {
                    building.add(execution, remaining, fee, basis);
                    break;
                }
                let closing = remaining.min(building.open_quantity);
                // The part of the fee that belongs to the closing quantity; the rest follows the
                // quantity that opens the next trade.
                let closing_fee = fee.map(|fee| {
                    if closing == remaining {
                        fee
                    } else {
                        share(fee, closing, remaining)
                    }
                });
                building.reduce(execution, closing, closing_fee, basis);
                fee = fee
                    .zip(closing_fee)
                    .map(|(fee, closing_fee)| fee - closing_fee);
                remaining -= closing;
                if building.open_quantity.is_zero()
                    && let Some(done) = current.take()
                {
                    trades.push(done.finish(TradeStatus::Closed));
                }
            }
        }
        if let Some(open) = current {
            trades.push(open.finish(TradeStatus::Open));
        }
    }
    trades.sort_by(|a, b| {
        (a.open_ts_ms, &a.key, &a.account_uid).cmp(&(b.open_ts_ms, &b.key, &b.account_uid))
    });
    trades
}

#[cfg(test)]
mod tests;
