#![allow(dead_code)]
//! Paper-trading execution engine: the order/fill/position state machine over
//! `PaperTradingSnapshot`. Ported from `PaperTradingEngine.swift`. Quotes arrive
//! via `process`; the engine itself never touches the network.

use std::collections::HashSet;

use crate::model::paper::{
    PaperAccount, PaperAccountMetrics, PaperAccountSettings, PaperClosedTrade,
    PaperCommissionConfiguration, PaperCurrency, PaperFill, PaperFillPriceSource, PaperInstrument,
    PaperJournalEntry, PaperOrder, PaperOrderChanges, PaperOrderEvent, PaperOrderEventKind,
    PaperOrderRequest, PaperOrderRole, PaperOrderSide, PaperOrderStatus, PaperOrderType,
    PaperPosition, PaperPositionSide, PaperQuote, PaperTimeInForce, PaperTradingError,
    PaperTradingSnapshot,
};

pub struct PaperTradingEngine {
    snapshot: PaperTradingSnapshot,
    next_id: u64,
    quote_max_age_ms: i64,
}

impl PaperTradingEngine {
    pub fn new(snapshot: PaperTradingSnapshot) -> Self {
        let next_id = snapshot
            .accounts
            .iter()
            .map(|account| account.id)
            .chain(snapshot.orders.iter().map(|order| order.id))
            .chain(snapshot.fills.iter().map(|fill| fill.id))
            .chain(snapshot.order_events.iter().map(|event| event.id))
            .chain(snapshot.closed_trades.iter().map(|trade| trade.id))
            .chain(snapshot.journal.iter().map(|entry| entry.id))
            // Ids at or above the legacy limit belong to records that arrived from another Mac.
            .filter(|id| *id < crate::sync::ids::LEGACY_LIMIT)
            .max()
            .map_or(1, |max| max + 1);
        Self {
            snapshot,
            next_id,
            quote_max_age_ms: 30_000,
        }
    }

    pub fn snapshot(&self) -> &PaperTradingSnapshot {
        &self.snapshot
    }

    pub fn into_snapshot(self) -> PaperTradingSnapshot {
        self.snapshot
    }

    // MARK: Accounts

    pub fn create_account(
        &mut self,
        name: &str,
        currency: PaperCurrency,
        initial_balance: f64,
        settings: PaperAccountSettings,
    ) -> Result<u64, PaperTradingError> {
        if initial_balance <= 0.0 {
            return Err(PaperTradingError::InvalidQuantity(
                "Initial balance must be greater than zero.".to_owned(),
            ));
        }
        let id = self.mint();
        let account = PaperAccount {
            id,
            name: if name.is_empty() {
                "Paper Trading".to_owned()
            } else {
                name.to_owned()
            },
            base_currency: currency,
            initial_balance,
            cash_balance: initial_balance,
            realized_pnl: 0.0,
            settings,
            created_at: now_ms(),
        };
        self.snapshot.accounts.push(account);
        self.snapshot.selected_account_id = Some(id);
        self.journal(
            id,
            format!("PAPER account created with {initial_balance} {currency:?}"),
        );
        Ok(id)
    }

    pub fn select_account(&mut self, id: u64) -> Result<(), PaperTradingError> {
        if !self
            .snapshot
            .accounts
            .iter()
            .any(|account| account.id == id)
        {
            return Err(PaperTradingError::AccountNotFound);
        }
        self.snapshot.selected_account_id = Some(id);
        Ok(())
    }

    pub fn update_settings(
        &mut self,
        account_id: u64,
        settings: PaperAccountSettings,
    ) -> Result<(), PaperTradingError> {
        let index = self
            .account_index(account_id)
            .ok_or(PaperTradingError::AccountNotFound)?;
        self.snapshot.accounts[index].settings = settings;
        self.journal(
            account_id,
            "PAPER account settings changed; changes apply prospectively".to_owned(),
        );
        Ok(())
    }

    pub fn reset_account(
        &mut self,
        id: u64,
        currency: PaperCurrency,
        initial_balance: f64,
        settings: PaperAccountSettings,
    ) -> Result<(), PaperTradingError> {
        let index = self
            .account_index(id)
            .ok_or(PaperTradingError::AccountNotFound)?;
        if initial_balance <= 0.0 {
            return Err(PaperTradingError::InvalidQuantity(
                "Initial balance must be greater than zero.".to_owned(),
            ));
        }
        let name = self.snapshot.accounts[index].name.clone();
        self.snapshot.accounts[index] = PaperAccount {
            id,
            name,
            base_currency: currency,
            initial_balance,
            cash_balance: initial_balance,
            realized_pnl: 0.0,
            settings,
            created_at: now_ms(),
        };
        self.snapshot.orders.retain(|order| order.account_id != id);
        self.snapshot.fills.retain(|fill| fill.account_id != id);
        self.snapshot
            .positions
            .retain(|position| position.account_id != id);
        self.snapshot
            .order_events
            .retain(|event| event.account_id != id);
        self.snapshot
            .closed_trades
            .retain(|trade| trade.account_id != id);
        self.snapshot.journal.retain(|entry| entry.account_id != id);
        self.journal(
            id,
            format!("PAPER account reset to {initial_balance} {currency:?}"),
        );
        Ok(())
    }

    pub fn delete_account(&mut self, id: u64) -> Result<(), PaperTradingError> {
        if !self
            .snapshot
            .accounts
            .iter()
            .any(|account| account.id == id)
        {
            return Err(PaperTradingError::AccountNotFound);
        }
        self.snapshot.accounts.retain(|account| account.id != id);
        self.snapshot.orders.retain(|order| order.account_id != id);
        self.snapshot.fills.retain(|fill| fill.account_id != id);
        self.snapshot
            .positions
            .retain(|position| position.account_id != id);
        self.snapshot
            .order_events
            .retain(|event| event.account_id != id);
        self.snapshot
            .closed_trades
            .retain(|trade| trade.account_id != id);
        self.snapshot.journal.retain(|entry| entry.account_id != id);
        if self.snapshot.selected_account_id == Some(id) {
            self.snapshot.selected_account_id =
                self.snapshot.accounts.first().map(|account| account.id);
        }
        Ok(())
    }

    // MARK: Orders

    pub fn submit(&mut self, request: PaperOrderRequest) -> Result<u64, PaperTradingError> {
        let account = self.account(request.account_id)?;
        self.validate(&request, &account)?;
        let timestamp = now_ms();
        let id = self.mint();
        let mut order = PaperOrder {
            id,
            account_id: request.account_id,
            instrument: request.instrument.clone(),
            side: request.side,
            kind: request.kind,
            original_quantity: request.quantity,
            filled_quantity: 0.0,
            average_fill_price: None,
            limit_price: request.limit_price,
            stop_price: request.stop_price,
            time_in_force: request.time_in_force,
            status: PaperOrderStatus::PendingSubmission,
            role: PaperOrderRole::Entry,
            parent_order_id: None,
            oco_group_id: None,
            stop_triggered: false,
            reserved_margin: 0.0,
            rejection_reason: None,
            created_at: timestamp,
            updated_at: timestamp,
        };
        self.snapshot.orders.push(order.clone());
        self.event(
            order.clone(),
            PaperOrderEventKind::Placed,
            format!(
                "{} {} {} {:?} order submitted",
                side_label(request.side),
                request.quantity,
                request.instrument.symbol,
                request.kind
            ),
        );

        match self.try_accept(
            &mut order,
            &account,
            timestamp,
            request.take_profit,
            request.stop_loss,
        ) {
            Ok(()) => Ok(id),
            Err(error) => {
                order.status = PaperOrderStatus::Rejected;
                order.rejection_reason = Some(error.message());
                order.updated_at = timestamp;
                self.replace(order.clone());
                self.event(order, PaperOrderEventKind::Rejected, error.message());
                Err(error)
            }
        }
    }

    fn try_accept(
        &mut self,
        order: &mut PaperOrder,
        account: &PaperAccount,
        timestamp: i64,
        take_profit: Option<f64>,
        stop_loss: Option<f64>,
    ) -> Result<(), PaperTradingError> {
        let (margin, needed) = self.funds_needed(order, account)?;
        let available = self.available_excluding(account.id, order.id);
        if needed > available {
            return Err(PaperTradingError::InsufficientFunds {
                required: needed,
                available,
            });
        }
        order.reserved_margin = margin;
        order.status = PaperOrderStatus::Working;
        order.updated_at = timestamp;
        self.replace(order.clone());
        self.event(
            order.clone(),
            PaperOrderEventKind::Accepted,
            "PAPER order accepted".to_owned(),
        );
        self.evaluate(order.id, Some((take_profit, stop_loss)))
    }

    pub fn cancel(&mut self, order_id: u64) -> Result<(), PaperTradingError> {
        let Some(mut order) = self
            .snapshot
            .orders
            .iter()
            .find(|order| order.id == order_id)
            .cloned()
        else {
            return Err(PaperTradingError::OrderNotFound);
        };
        if !order.status.is_working() {
            return Err(PaperTradingError::InvalidTransition);
        }
        order.status = PaperOrderStatus::PendingCancel;
        order.updated_at = now_ms();
        self.replace(order.clone());
        order.status = PaperOrderStatus::Canceled;
        order.reserved_margin = 0.0;
        self.replace(order.clone());
        self.event(
            order,
            PaperOrderEventKind::Canceled,
            "PAPER order canceled".to_owned(),
        );
        Ok(())
    }

    pub fn modify(
        &mut self,
        order_id: u64,
        changes: PaperOrderChanges,
    ) -> Result<(), PaperTradingError> {
        let Some(mut order) = self
            .snapshot
            .orders
            .iter()
            .find(|order| order.id == order_id)
            .cloned()
        else {
            return Err(PaperTradingError::OrderNotFound);
        };
        if !order.status.is_working() {
            return Err(PaperTradingError::InvalidTransition);
        }
        if let Some(quantity) = changes.quantity {
            if quantity < order.filled_quantity || quantity <= 0.0 {
                return Err(PaperTradingError::InvalidQuantity(
                    "Quantity cannot be below the already-filled quantity.".to_owned(),
                ));
            }
            validate_step(quantity, order.instrument.quantity_increment, false)?;
            order.original_quantity = quantity;
        }
        if let Some(price) = changes.limit_price {
            validate_price(price, &order.instrument)?;
            order.limit_price = Some(price);
        }
        if let Some(price) = changes.stop_price {
            validate_price(price, &order.instrument)?;
            order.stop_price = Some(price);
        }
        order.updated_at = now_ms();
        let account = self.account(order.account_id)?;
        // The same check as at submission, so a modification cannot grow an order past what the
        // account could have placed. The order's own current reservation is not held against it.
        let (margin, needed) = self.funds_needed(&order, &account)?;
        let available = self.available_excluding(account.id, order.id);
        if needed > available {
            return Err(PaperTradingError::InsufficientFunds {
                required: needed,
                available,
            });
        }
        order.reserved_margin = margin;
        self.replace(order.clone());
        self.event(
            order.clone(),
            PaperOrderEventKind::Modified,
            "PAPER order modified".to_owned(),
        );
        match self.evaluate(order.id, None) {
            Err(error @ PaperTradingError::InsufficientFunds { .. }) => {
                self.reject_working(order.id, &error);
                Err(error)
            }
            other => other,
        }
    }

    /// Feed a live quote. Only quotes that could have moved an order are persisted.
    pub fn process(&mut self, quote: PaperQuote) -> Result<(), PaperTradingError> {
        self.process_except(quote, &HashSet::new())
    }

    /// Like `process`, but never fills the orders of the accounts in `skip`: an account that
    /// another Mac runs is only shown here, so a working order is not filled twice.
    pub fn process_except(
        &mut self,
        quote: PaperQuote,
        skip: &HashSet<u64>,
    ) -> Result<(), PaperTradingError> {
        if let Some(previous) = self.snapshot.quotes.get(&quote.instrument_key)
            && quote.timestamp < previous.timestamp
        {
            return Ok(());
        }
        let key = quote.instrument_key.clone();
        self.snapshot.quotes.insert(key.clone(), quote);
        let ids: Vec<u64> = {
            let mut working: Vec<&PaperOrder> = self
                .snapshot
                .orders
                .iter()
                .filter(|order| {
                    order.instrument.key == key
                        && order.status.is_working()
                        && !skip.contains(&order.account_id)
                })
                .collect();
            working.sort_by_key(|order| order.created_at);
            working.iter().map(|order| order.id).collect()
        };
        // One order's failure must not leave the rest of the quote's orders unevaluated.
        let mut first_error = None;
        for id in ids {
            match self.evaluate(id, None) {
                Ok(()) => {}
                Err(error @ PaperTradingError::InsufficientFunds { .. }) => {
                    // Reserved at a better price than it filled at (a gap through a stop, slippage,
                    // a commission): the order is refused, not filled into a deficit.
                    self.reject_working(id, &error);
                }
                Err(error) => {
                    first_error.get_or_insert(error);
                }
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    /// Ends a working order that can no longer be afforded, with an audit row.
    fn reject_working(&mut self, order_id: u64, error: &PaperTradingError) {
        let Some(mut order) = self
            .snapshot
            .orders
            .iter()
            .find(|order| order.id == order_id && order.status.is_working())
            .cloned()
        else {
            return;
        };
        order.status = PaperOrderStatus::Rejected;
        order.rejection_reason = Some(error.message());
        order.reserved_margin = 0.0;
        order.updated_at = now_ms();
        self.replace(order.clone());
        self.event(order, PaperOrderEventKind::Rejected, error.message());
    }

    // MARK: Metrics

    pub fn metrics(&self, account_id: u64) -> PaperAccountMetrics {
        let Some(account) = self
            .snapshot
            .accounts
            .iter()
            .find(|account| account.id == account_id)
        else {
            return PaperAccountMetrics::default();
        };
        let positions: Vec<&PaperPosition> = self
            .snapshot
            .positions
            .iter()
            .filter(|position| position.account_id == account_id)
            .collect();
        let unrealized: f64 = positions
            .iter()
            .map(|position| self.unrealized_pnl(position))
            .sum();
        let position_margin: f64 = positions
            .iter()
            .map(|position| {
                let mark = self
                    .liquidation_price(position)
                    .unwrap_or(position.average_entry_price);
                let leverage = account
                    .settings
                    .leverage
                    .leverage(position.instrument.asset_class)
                    .max(1.0);
                position.quantity() * mark * position.instrument.contract_multiplier / leverage
            })
            .sum();
        let orders_margin: f64 = self
            .snapshot
            .orders
            .iter()
            .filter(|order| order.account_id == account_id && order.status.is_working())
            .map(|order| order.reserved_margin)
            .sum();
        let equity = account.cash_balance + unrealized;
        let available = (equity - position_margin - orders_margin).max(0.0);
        PaperAccountMetrics {
            balance: account.cash_balance,
            equity,
            realized_pnl: account.realized_pnl,
            unrealized_pnl: unrealized,
            position_margin,
            orders_margin,
            available_funds: available,
            margin_buffer: if equity == 0.0 {
                0.0
            } else {
                available / equity.abs()
            },
        }
    }

    pub fn unrealized_pnl(&self, position: &PaperPosition) -> f64 {
        let Some(liquidation) = self.liquidation_price(position) else {
            return 0.0;
        };
        let difference = if position.signed_quantity >= 0.0 {
            liquidation - position.average_entry_price
        } else {
            position.average_entry_price - liquidation
        };
        difference * position.quantity() * position.instrument.point_value
    }

    // MARK: Execution

    fn evaluate(
        &mut self,
        order_id: u64,
        bracket: Option<(Option<f64>, Option<f64>)>,
    ) -> Result<(), PaperTradingError> {
        let Some(mut order) = self
            .snapshot
            .orders
            .iter()
            .find(|order| order.id == order_id)
            .cloned()
        else {
            return Ok(());
        };
        if !order.status.is_working() {
            return Ok(());
        }
        let Some(quote) = self.snapshot.quotes.get(&order.instrument.key).cloned() else {
            if order.kind == PaperOrderType::Market {
                return Err(PaperTradingError::NoMarketData);
            }
            return Ok(());
        };
        if !quote.is_market_open {
            return Ok(());
        }
        if now_ms() - quote.timestamp > self.quote_max_age_ms {
            if order.kind == PaperOrderType::Market {
                return Err(PaperTradingError::StaleMarketData);
            }
            return Ok(());
        }
        let Some((executable, source)) = quote.executable_price(order.side) else {
            return Ok(());
        };

        let mut fill_price = None;
        match order.kind {
            PaperOrderType::Market => fill_price = Some(executable),
            PaperOrderType::Limit => {
                if limit_eligible(&order, executable) {
                    fill_price = Some(better_price(&order, executable));
                }
            }
            PaperOrderType::Stop => {
                if stop_eligible(&order, executable) {
                    fill_price = Some(executable);
                }
            }
            PaperOrderType::StopLimit => {
                if !order.stop_triggered && stop_eligible(&order, executable) {
                    order.stop_triggered = true;
                    order.updated_at = now_ms();
                    self.replace(order.clone());
                    self.event(
                        order.clone(),
                        PaperOrderEventKind::Modified,
                        "Stop triggered; PAPER stop-limit activated".to_owned(),
                    );
                }
                if order.stop_triggered && limit_eligible(&order, executable) {
                    fill_price = Some(better_price(&order, executable));
                }
            }
        }
        let Some(mut price) = fill_price else {
            return Ok(());
        };
        let ticks = self
            .account(order.account_id)
            .map(|account| account.settings.slippage_ticks)
            .unwrap_or(0.0);
        if ticks != 0.0 {
            let direction = if order.side == PaperOrderSide::Buy {
                1.0
            } else {
                -1.0
            };
            price += direction * ticks * order.instrument.tick_size;
        }
        let quantity = order.remaining_quantity();
        self.fill(&mut order, quantity, price, source, bracket)
    }

    fn fill(
        &mut self,
        order: &mut PaperOrder,
        quantity: f64,
        price: f64,
        source: PaperFillPriceSource,
        bracket: Option<(Option<f64>, Option<f64>)>,
    ) -> Result<(), PaperTradingError> {
        if quantity <= 0.0 || !order.status.is_working() {
            return Ok(());
        }
        let account = self.account(order.account_id)?;
        self.check_fill_funds(order, &account, quantity, price)?;
        let commission = commission(
            &account.settings.commission,
            quantity,
            price,
            &order.instrument,
        );
        let fill = PaperFill {
            id: self.mint(),
            order_id: order.id,
            account_id: order.account_id,
            instrument: order.instrument.clone(),
            side: order.side,
            quantity,
            price,
            commission,
            price_source: source,
            timestamp: now_ms(),
        };
        let prior_filled = order.filled_quantity;
        order.filled_quantity += quantity;
        order.average_fill_price = Some(
            (order.average_fill_price.unwrap_or(0.0) * prior_filled + price * quantity)
                / order.filled_quantity,
        );
        order.status = if order.remaining_quantity() == 0.0 {
            PaperOrderStatus::Filled
        } else {
            PaperOrderStatus::PartiallyFilled
        };
        order.reserved_margin = 0.0;
        order.updated_at = now_ms();
        self.replace(order.clone());
        self.snapshot.fills.push(fill.clone());

        let realized = self.apply_position(fill.clone());
        let account_index = self
            .account_index(order.account_id)
            .ok_or(PaperTradingError::AccountNotFound)?;
        self.snapshot.accounts[account_index].cash_balance += realized - commission;
        self.snapshot.accounts[account_index].realized_pnl += realized - commission;

        let kind = if order.status == PaperOrderStatus::Filled {
            PaperOrderEventKind::Filled
        } else {
            PaperOrderEventKind::PartiallyFilled
        };
        self.event(
            order.clone(),
            kind,
            format!(
                "Filled {} {} @ {} [{:?}]; commission {}",
                quantity, order.instrument.symbol, price, source, commission
            ),
        );
        if realized != 0.0 {
            self.journal(
                order.account_id,
                format!("Realized gross P&L {} before commission", realized),
            );
        }
        if order.status == PaperOrderStatus::Filled {
            let group = order.oco_group_id;
            self.cancel_oco_counterpart(order.id, group);
            if order.role == PaperOrderRole::Entry
                && let Some((take_profit, stop_loss)) = bracket
            {
                self.create_bracket_children(order, take_profit, stop_loss);
            }
        }
        Ok(())
    }

    /// Apply a fill to the position it touches, returning the realized gross P&L.
    fn apply_position(&mut self, fill: PaperFill) -> f64 {
        let delta = if fill.side == PaperOrderSide::Buy {
            fill.quantity
        } else {
            -fill.quantity
        };
        let Some(index) = self.snapshot.positions.iter().position(|position| {
            position.account_id == fill.account_id && position.instrument.key == fill.instrument.key
        }) else {
            self.snapshot.positions.push(PaperPosition {
                account_id: fill.account_id,
                instrument: fill.instrument.clone(),
                signed_quantity: delta,
                average_entry_price: fill.price,
                realized_gross_pnl: 0.0,
                commissions: fill.commission,
                opened_at: fill.timestamp,
                updated_at: fill.timestamp,
            });
            return 0.0;
        };

        let mut position = self.snapshot.positions[index].clone();
        let old_signed = position.signed_quantity;
        let same_direction =
            (old_signed >= 0.0 && delta > 0.0) || (old_signed <= 0.0 && delta < 0.0);
        if same_direction {
            let total = old_signed.abs() + delta.abs();
            position.average_entry_price = (position.average_entry_price * old_signed.abs()
                + fill.price * delta.abs())
                / total;
            position.signed_quantity += delta;
            position.commissions += fill.commission;
            position.updated_at = fill.timestamp;
            self.snapshot.positions[index] = position;
            return 0.0;
        }

        let closed_quantity = old_signed.abs().min(delta.abs());
        let gross = (if old_signed > 0.0 {
            fill.price - position.average_entry_price
        } else {
            position.average_entry_price - fill.price
        }) * closed_quantity
            * position.instrument.point_value;
        let allocated_entry_commission = if position.quantity() == 0.0 {
            0.0
        } else {
            position.commissions * closed_quantity / position.quantity()
        };
        let closing_commission = if fill.quantity == 0.0 {
            0.0
        } else {
            fill.commission * closed_quantity / fill.quantity
        };
        let opening_commission = fill.commission - closing_commission;
        let trade_commission = allocated_entry_commission + closing_commission;

        let trade_id = self.mint();
        self.snapshot.closed_trades.push(PaperClosedTrade {
            id: trade_id,
            account_id: fill.account_id,
            instrument: fill.instrument.clone(),
            side: if old_signed > 0.0 {
                PaperPositionSide::Long
            } else {
                PaperPositionSide::Short
            },
            entry_timestamp: position.opened_at,
            exit_timestamp: fill.timestamp,
            entry_price: position.average_entry_price,
            exit_price: fill.price,
            quantity: closed_quantity,
            gross_pnl: gross,
            commission: trade_commission,
        });

        position.realized_gross_pnl += gross;
        position.commissions -= allocated_entry_commission;
        position.signed_quantity += delta;
        position.updated_at = fill.timestamp;
        if position.signed_quantity == 0.0 {
            self.snapshot.positions.remove(index);
        } else if (old_signed > 0.0) != (position.signed_quantity > 0.0) {
            position.average_entry_price = fill.price;
            position.opened_at = fill.timestamp;
            position.commissions = opening_commission;
            self.snapshot.positions[index] = position;
        } else {
            self.snapshot.positions[index] = position;
        }
        self.rebalance_protection(fill.account_id, &fill.instrument.key);
        gross
    }

    fn create_bracket_children(
        &mut self,
        parent: &PaperOrder,
        take_profit: Option<f64>,
        stop_loss: Option<f64>,
    ) {
        if take_profit.is_none() && stop_loss.is_none() {
            return;
        }
        // Protection is sized to the position the entry left, on the entry's side: an entry that
        // only reduced or reversed an existing position must not arm an exit larger than it.
        let protected = self
            .snapshot
            .positions
            .iter()
            .find(|position| {
                position.account_id == parent.account_id
                    && position.instrument.key == parent.instrument.key
            })
            .filter(|position| {
                (position.signed_quantity > 0.0) == (parent.side == PaperOrderSide::Buy)
            })
            .map_or(0.0, |position| {
                position.quantity().min(parent.filled_quantity)
            });
        if protected <= 0.0 {
            return;
        }
        let group = self.mint();
        if let Some(price) = take_profit {
            self.append_bracket_child(
                parent,
                group,
                PaperOrderType::Limit,
                price,
                PaperOrderRole::TakeProfit,
                protected,
            );
        }
        if let Some(price) = stop_loss {
            self.append_bracket_child(
                parent,
                group,
                PaperOrderType::Stop,
                price,
                PaperOrderRole::StopLoss,
                protected,
            );
        }
    }

    fn append_bracket_child(
        &mut self,
        parent: &PaperOrder,
        group: u64,
        kind: PaperOrderType,
        price: f64,
        role: PaperOrderRole,
        quantity: f64,
    ) {
        let side = if parent.side == PaperOrderSide::Buy {
            PaperOrderSide::Sell
        } else {
            PaperOrderSide::Buy
        };
        let child = PaperOrder {
            id: self.mint(),
            account_id: parent.account_id,
            instrument: parent.instrument.clone(),
            side,
            kind,
            original_quantity: quantity,
            filled_quantity: 0.0,
            average_fill_price: None,
            limit_price: (kind == PaperOrderType::Limit).then_some(price),
            stop_price: (kind == PaperOrderType::Stop).then_some(price),
            time_in_force: PaperTimeInForce::GoodTilCanceled,
            status: PaperOrderStatus::Working,
            role,
            parent_order_id: Some(parent.id),
            oco_group_id: Some(group),
            stop_triggered: false,
            reserved_margin: 0.0,
            rejection_reason: None,
            created_at: now_ms(),
            updated_at: now_ms(),
        };
        self.snapshot.orders.push(child.clone());
        self.event(
            child,
            PaperOrderEventKind::Accepted,
            format!("PAPER {role:?} protection activated"),
        );
    }

    fn cancel_oco_counterpart(&mut self, order_id: u64, group: Option<u64>) {
        let Some(group) = group else {
            return;
        };
        let indices: Vec<usize> = self
            .snapshot
            .orders
            .iter()
            .enumerate()
            .filter(|(_, order)| {
                order.oco_group_id == Some(group)
                    && order.id != order_id
                    && order.status.is_working()
            })
            .map(|(index, _)| index)
            .collect();
        for index in indices {
            self.snapshot.orders[index].status = PaperOrderStatus::Canceled;
            self.snapshot.orders[index].reserved_margin = 0.0;
            self.snapshot.orders[index].updated_at = now_ms();
            let order = self.snapshot.orders[index].clone();
            self.event(
                order,
                PaperOrderEventKind::Canceled,
                "PAPER OCO counterpart canceled".to_owned(),
            );
        }
    }

    /// Keeps stops and take-profits honest after the position changed. Protection that no longer
    /// reduces the position (it was closed, or reversed so the old exit now adds to the new side)
    /// is canceled; protection larger than what is left is shrunk to it, never grown. Every
    /// change is audited. A protective order can therefore at worst close the position.
    fn rebalance_protection(&mut self, account_id: u64, instrument_key: &str) {
        let signed = self
            .snapshot
            .positions
            .iter()
            .find(|position| {
                position.account_id == account_id && position.instrument.key == instrument_key
            })
            .map_or(0.0, |position| position.signed_quantity);
        let size = signed.abs();
        let indices: Vec<usize> = self
            .snapshot
            .orders
            .iter()
            .enumerate()
            .filter(|(_, order)| {
                order.account_id == account_id
                    && order.instrument.key == instrument_key
                    && order.role != PaperOrderRole::Entry
                    && order.status.is_working()
            })
            .map(|(index, _)| index)
            .collect();
        for index in indices {
            let order = &self.snapshot.orders[index];
            let reduces = (signed > 0.0 && order.side == PaperOrderSide::Sell)
                || (signed < 0.0 && order.side == PaperOrderSide::Buy);
            if !reduces {
                let message = if size == 0.0 {
                    "PAPER protection canceled; the position is closed"
                } else {
                    "PAPER protection canceled; the position reversed"
                };
                self.snapshot.orders[index].status = PaperOrderStatus::Canceled;
                self.snapshot.orders[index].reserved_margin = 0.0;
                self.snapshot.orders[index].updated_at = now_ms();
                let order = self.snapshot.orders[index].clone();
                self.event(order, PaperOrderEventKind::Canceled, message.to_owned());
            } else if order.original_quantity > order.filled_quantity + size {
                self.snapshot.orders[index].original_quantity =
                    self.snapshot.orders[index].filled_quantity + size;
                self.snapshot.orders[index].updated_at = now_ms();
                let order = self.snapshot.orders[index].clone();
                self.event(
                    order,
                    PaperOrderEventKind::Modified,
                    format!("PAPER protection resized to the remaining {size}"),
                );
            }
        }
    }

    // MARK: Validation / calculations

    fn validate(
        &self,
        request: &PaperOrderRequest,
        account: &PaperAccount,
    ) -> Result<(), PaperTradingError> {
        if request.quantity < request.instrument.minimum_quantity {
            return Err(PaperTradingError::InvalidQuantity(format!(
                "Quantity must be at least {}.",
                request.instrument.minimum_quantity
            )));
        }
        validate_step(
            request.quantity,
            request.instrument.quantity_increment,
            false,
        )?;
        if request.instrument.quote_currency != account.base_currency
            && !(matches!(
                request.instrument.quote_currency,
                PaperCurrency::Usdt | PaperCurrency::Usdc
            ) && account.base_currency == PaperCurrency::Usd)
        {
            return Err(PaperTradingError::UnsupportedCurrencyConversion {
                from: request.instrument.quote_currency,
                to: account.base_currency,
            });
        }
        match request.kind {
            PaperOrderType::Market => {}
            PaperOrderType::Limit => {
                let Some(price) = request.limit_price else {
                    return Err(PaperTradingError::InvalidPrice(
                        "A limit price is required.".to_owned(),
                    ));
                };
                validate_price(price, &request.instrument)?;
            }
            PaperOrderType::Stop => {
                let Some(price) = request.stop_price else {
                    return Err(PaperTradingError::InvalidPrice(
                        "A stop price is required.".to_owned(),
                    ));
                };
                validate_price(price, &request.instrument)?;
            }
            PaperOrderType::StopLimit => {
                let (Some(limit), Some(stop)) = (request.limit_price, request.stop_price) else {
                    return Err(PaperTradingError::InvalidPrice(
                        "Stop and limit prices are required.".to_owned(),
                    ));
                };
                validate_price(limit, &request.instrument)?;
                validate_price(stop, &request.instrument)?;
            }
        }
        if let Some(price) = request.take_profit {
            validate_price(price, &request.instrument)?;
        }
        if let Some(price) = request.stop_loss {
            validate_price(price, &request.instrument)?;
        }
        Ok(())
    }

    /// The price an entry order is sized at: its limit or stop, else what the quote offers.
    fn reference_price(&self, order: &PaperOrder) -> Result<f64, PaperTradingError> {
        if matches!(
            order.kind,
            PaperOrderType::Limit | PaperOrderType::StopLimit
        ) && let Some(limit) = order.limit_price
        {
            return Ok(limit);
        }
        if order.kind == PaperOrderType::Stop
            && let Some(stop) = order.stop_price
        {
            return Ok(stop);
        }
        self.snapshot
            .quotes
            .get(&order.instrument.key)
            .and_then(|quote| quote.executable_price(order.side))
            .map(|(price, _)| price)
            .ok_or(PaperTradingError::NoMarketData)
    }

    /// How much of the open position this order may count as reducing: the position, less what
    /// older working entry orders on the same side already counted. Without that, any number of
    /// one-unit exits against a one-unit position would each reserve nothing, and the second to
    /// fill would open an unfunded short.
    fn reducible_quantity(&self, order: &PaperOrder) -> f64 {
        let signed = self
            .snapshot
            .positions
            .iter()
            .find(|position| {
                position.account_id == order.account_id
                    && position.instrument.key == order.instrument.key
            })
            .map_or(0.0, |position| position.signed_quantity);
        let reduces = (signed > 0.0 && order.side == PaperOrderSide::Sell)
            || (signed < 0.0 && order.side == PaperOrderSide::Buy);
        if !reduces {
            return 0.0;
        }
        let claimed: f64 = self
            .snapshot
            .orders
            .iter()
            .filter(|other| {
                other.id != order.id
                    && other.account_id == order.account_id
                    && other.instrument.key == order.instrument.key
                    && other.side == order.side
                    && other.role == PaperOrderRole::Entry
                    && other.status.is_working()
                    && (other.created_at, other.id) < (order.created_at, order.id)
            })
            .map(PaperOrder::remaining_quantity)
            .sum();
        (signed.abs() - claimed).max(0.0)
    }

    fn leverage(account: &PaperAccount, instrument: &PaperInstrument) -> f64 {
        account
            .settings
            .leverage
            .leverage(instrument.asset_class)
            .max(1.0)
    }

    /// The margin an entry order reserves, and the funds it needs in all: the margin plus its
    /// commission, when it adds exposure. An order that only closes needs neither, so a fully
    /// invested account can always get out.
    fn funds_needed(
        &self,
        order: &PaperOrder,
        account: &PaperAccount,
    ) -> Result<(f64, f64), PaperTradingError> {
        if order.role != PaperOrderRole::Entry {
            return Ok((0.0, 0.0));
        }
        let price = self.reference_price(order)?;
        let increase = (order.remaining_quantity() - self.reducible_quantity(order)).max(0.0);
        let margin = increase * price * order.instrument.contract_multiplier
            / Self::leverage(account, &order.instrument);
        if increase == 0.0 {
            return Ok((margin, margin));
        }
        let fee = commission(
            &account.settings.commission,
            order.remaining_quantity(),
            price,
            &order.instrument,
        );
        Ok((margin, margin + fee))
    }

    /// Funds free for one order: equity less position margin and every other order's reservation.
    fn available_excluding(&self, account_id: u64, order_id: u64) -> f64 {
        let metrics = self.metrics(account_id);
        let own = self
            .snapshot
            .orders
            .iter()
            .find(|order| order.id == order_id && order.status.is_working())
            .map_or(0.0, |order| order.reserved_margin);
        (metrics.equity - metrics.position_margin - (metrics.orders_margin - own)).max(0.0)
    }

    /// The last check before a fill: the order was reserved at its limit or stop, but fills at the
    /// executable price, plus slippage and commission. A fill that adds exposure the account
    /// cannot carry is refused rather than booked into a deficit.
    fn check_fill_funds(
        &self,
        order: &PaperOrder,
        account: &PaperAccount,
        quantity: f64,
        price: f64,
    ) -> Result<(), PaperTradingError> {
        if order.role != PaperOrderRole::Entry {
            return Ok(());
        }
        let signed = self
            .snapshot
            .positions
            .iter()
            .find(|position| {
                position.account_id == order.account_id
                    && position.instrument.key == order.instrument.key
            })
            .map_or(0.0, |position| position.signed_quantity);
        let reduces = (signed > 0.0 && order.side == PaperOrderSide::Sell)
            || (signed < 0.0 && order.side == PaperOrderSide::Buy);
        let increase = (quantity - if reduces { signed.abs() } else { 0.0 }).max(0.0);
        if increase == 0.0 {
            return Ok(());
        }
        let margin = increase * price * order.instrument.contract_multiplier
            / Self::leverage(account, &order.instrument);
        let needed = margin
            + commission(
                &account.settings.commission,
                quantity,
                price,
                &order.instrument,
            );
        let available = self.available_excluding(account.id, order.id);
        // A fill within a cent of the funds is not a deficit worth refusing over.
        if needed > available + 0.005 {
            return Err(PaperTradingError::InsufficientFunds {
                required: needed,
                available,
            });
        }
        Ok(())
    }

    fn liquidation_price(&self, position: &PaperPosition) -> Option<f64> {
        let quote = self.snapshot.quotes.get(&position.instrument.key)?;
        if position.signed_quantity >= 0.0 {
            quote.bid.or(quote.last)
        } else {
            quote.ask.or(quote.last)
        }
    }

    fn account(&self, id: u64) -> Result<PaperAccount, PaperTradingError> {
        self.snapshot
            .accounts
            .iter()
            .find(|account| account.id == id)
            .cloned()
            .ok_or(PaperTradingError::AccountNotFound)
    }

    fn account_index(&self, id: u64) -> Option<usize> {
        self.snapshot
            .accounts
            .iter()
            .position(|account| account.id == id)
    }

    fn replace(&mut self, order: PaperOrder) {
        if let Some(index) = self
            .snapshot
            .orders
            .iter()
            .position(|candidate| candidate.id == order.id)
        {
            self.snapshot.orders[index] = order;
        }
    }

    fn event(&mut self, order: PaperOrder, kind: PaperOrderEventKind, message: String) {
        let account_id = order.account_id;
        let event_id = self.mint();
        self.snapshot.order_events.push(PaperOrderEvent {
            id: event_id,
            order_id: order.id,
            account_id,
            kind,
            timestamp: now_ms(),
            message: message.clone(),
            order,
        });
        self.journal(account_id, message);
    }

    fn journal(&mut self, account_id: u64, message: String) {
        let entry_id = self.mint();
        self.snapshot.journal.push(PaperJournalEntry {
            id: entry_id,
            account_id,
            timestamp: now_ms(),
            message,
        });
    }

    fn mint(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
}

// MARK: Pure helpers

fn side_label(side: PaperOrderSide) -> &'static str {
    match side {
        PaperOrderSide::Buy => "BUY",
        PaperOrderSide::Sell => "SELL",
    }
}

fn validate_price(value: f64, instrument: &PaperInstrument) -> Result<(), PaperTradingError> {
    if value <= 0.0 {
        return Err(PaperTradingError::InvalidPrice(
            "Price must be greater than zero.".to_owned(),
        ));
    }
    validate_step(value, instrument.tick_size, true)
}

fn validate_step(value: f64, increment: f64, is_price: bool) -> Result<(), PaperTradingError> {
    if increment <= 0.0 {
        return Ok(());
    }
    if !crate::model::paper::ticket::is_aligned(value, increment) {
        return if is_price {
            Err(PaperTradingError::InvalidPrice(format!(
                "Price must align to tick size {increment}."
            )))
        } else {
            Err(PaperTradingError::InvalidQuantity(format!(
                "Quantity must align to increment {increment}."
            )))
        };
    }
    Ok(())
}

fn commission(
    config: &PaperCommissionConfiguration,
    quantity: f64,
    price: f64,
    instrument: &PaperInstrument,
) -> f64 {
    match config {
        PaperCommissionConfiguration::None => 0.0,
        PaperCommissionConfiguration::FixedPerOrder { value } => *value,
        PaperCommissionConfiguration::Percentage { value } => {
            quantity * price * instrument.contract_multiplier * value / 100.0
        }
        PaperCommissionConfiguration::PerContract { value } => quantity * value,
    }
}

fn stop_eligible(order: &PaperOrder, executable: f64) -> bool {
    order.stop_price.is_some_and(|stop| {
        if order.side == PaperOrderSide::Buy {
            executable >= stop
        } else {
            executable <= stop
        }
    })
}

fn limit_eligible(order: &PaperOrder, executable: f64) -> bool {
    order.limit_price.is_some_and(|limit| {
        if order.side == PaperOrderSide::Buy {
            executable <= limit
        } else {
            executable >= limit
        }
    })
}

fn better_price(order: &PaperOrder, executable: f64) -> f64 {
    match order.limit_price {
        Some(limit) if order.side == PaperOrderSide::Buy => executable.min(limit),
        Some(limit) => executable.max(limit),
        None => executable,
    }
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis() as i64)
}

#[cfg(test)]
mod step_tests {
    use super::*;

    #[test]
    fn a_large_price_on_a_tiny_tick_is_on_the_tick() {
        // The default crypto tick: 83,105.43 is 8.3 trillion steps, and its float noise is not a
        // fraction of a step.
        for price in [83_105.43, 118_150.99, 67_432.19] {
            assert!(validate_step(price, 0.000_000_01, true).is_ok(), "{price}");
        }
        assert!(matches!(
            validate_step(83_105.000_000_001, 0.000_000_01, true),
            Err(PaperTradingError::InvalidPrice(_))
        ));
        assert!(matches!(
            validate_step(0.0015, 0.001, false),
            Err(PaperTradingError::InvalidQuantity(_))
        ));
        assert!(validate_step(0.5, 0.000_000_01, false).is_ok());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instrument(symbol: &str) -> PaperInstrument {
        PaperInstrument::chart(symbol, symbol, crate::model::DataSource::Binance)
    }

    fn quote(key: &str, last: f64) -> PaperQuote {
        PaperQuote {
            instrument_key: key.to_owned(),
            bid: Some(last - 0.5),
            ask: Some(last + 0.5),
            last: Some(last),
            timestamp: now_ms(),
            is_market_open: true,
        }
    }

    fn request(account_id: u64, kind: PaperOrderType, quantity: f64) -> PaperOrderRequest {
        PaperOrderRequest {
            account_id,
            instrument: instrument("BTCUSDT"),
            side: PaperOrderSide::Buy,
            kind,
            quantity,
            limit_price: (kind == PaperOrderType::Limit).then_some(100.0),
            stop_price: None,
            time_in_force: PaperTimeInForce::GoodTilCanceled,
            take_profit: None,
            stop_loss: None,
        }
    }

    fn engine_with_account() -> (PaperTradingEngine, u64) {
        let mut engine = PaperTradingEngine::new(PaperTradingSnapshot::default());
        let id = engine
            .create_account(
                "Test",
                PaperCurrency::Usd,
                100_000.0,
                PaperAccountSettings::default(),
            )
            .unwrap();
        (engine, id)
    }

    #[test]
    fn counter_ignores_ids_that_arrived_from_another_mac() {
        let (engine, _) = engine_with_account();
        let mut snapshot = engine.snapshot().clone();
        snapshot.accounts[0].id = crate::sync::ids::GLOBAL_BASE | (7 << 24) | 3;
        let mut engine = PaperTradingEngine::new(snapshot);
        let local = engine
            .create_account(
                "Local",
                PaperCurrency::Usd,
                1_000.0,
                PaperAccountSettings::default(),
            )
            .unwrap();
        assert!(local < crate::sync::ids::LEGACY_LIMIT, "minted {local}");
    }

    #[test]
    fn market_order_fills_at_quote_price() {
        let (mut engine, account) = engine_with_account();
        engine.process(quote("Binance:BTCUSDT", 100.0)).unwrap();
        let order_id = engine
            .submit(request(account, PaperOrderType::Market, 1.0))
            .unwrap();

        let order = engine
            .snapshot()
            .orders
            .iter()
            .find(|o| o.id == order_id)
            .unwrap();
        assert_eq!(order.status, PaperOrderStatus::Filled);
        assert_eq!(order.filled_quantity, 1.0);
        assert_eq!(order.average_fill_price, Some(100.5)); // ask, not last
        let position = engine
            .snapshot()
            .positions
            .iter()
            .find(|p| p.instrument.key == "Binance:BTCUSDT")
            .unwrap();
        assert_eq!(position.signed_quantity, 1.0);
    }

    #[test]
    fn limit_order_waits_for_a_crossing_price() {
        let (mut engine, account) = engine_with_account();
        engine.process(quote("Binance:BTCUSDT", 110.0)).unwrap();
        // Buy limit at 100; executable 110.5 is above the limit → no fill.
        let order_id = engine
            .submit(request(account, PaperOrderType::Limit, 1.0))
            .unwrap();
        assert_eq!(
            engine
                .snapshot()
                .orders
                .iter()
                .find(|o| o.id == order_id)
                .unwrap()
                .status,
            PaperOrderStatus::Working
        );

        // Price drops to 99 → executable 99.5 crosses the buy limit → fills at 99.5.
        engine.process(quote("Binance:BTCUSDT", 99.0)).unwrap();
        let order = engine
            .snapshot()
            .orders
            .iter()
            .find(|o| o.id == order_id)
            .unwrap();
        assert_eq!(order.status, PaperOrderStatus::Filled);
        assert_eq!(order.average_fill_price, Some(99.5));
    }

    #[test]
    fn buy_then_sell_realizes_pnl() {
        let (mut engine, account) = engine_with_account();
        engine.process(quote("Binance:BTCUSDT", 100.0)).unwrap();
        engine
            .submit(request(account, PaperOrderType::Market, 2.0))
            .unwrap();
        engine.process(quote("Binance:BTCUSDT", 150.0)).unwrap();
        engine
            .submit(PaperOrderRequest {
                side: PaperOrderSide::Sell,
                ..request(account, PaperOrderType::Market, 1.0)
            })
            .unwrap();

        let account = engine
            .snapshot()
            .accounts
            .iter()
            .find(|a| a.id == account)
            .unwrap();
        // Sold 1 at bid 149.5 (entry avg 100.5) → gross 49.0, no commission.
        assert!((account.realized_pnl - 49.0).abs() < 1e-9);
        let position = engine
            .snapshot()
            .positions
            .iter()
            .find(|p| p.instrument.key == "Binance:BTCUSDT")
            .unwrap();
        assert_eq!(position.signed_quantity, 1.0);
    }

    #[test]
    fn insufficient_funds_rejects_the_order() {
        let (mut engine, account) = engine_with_account();
        engine.process(quote("Binance:BTCUSDT", 100.0)).unwrap();
        let result = engine.submit(request(account, PaperOrderType::Market, 10_000.0));
        assert!(matches!(
            result,
            Err(PaperTradingError::InsufficientFunds { .. })
        ));
        assert!(
            engine.snapshot().orders.is_empty()
                || engine.snapshot().orders[0].status == PaperOrderStatus::Rejected
        );
    }

    #[test]
    fn cancel_only_works_on_a_live_order() {
        let (mut engine, account) = engine_with_account();
        engine.process(quote("Binance:BTCUSDT", 110.0)).unwrap();
        let order_id = engine
            .submit(request(account, PaperOrderType::Limit, 1.0))
            .unwrap();
        engine.cancel(order_id).unwrap();
        assert_eq!(
            engine
                .snapshot()
                .orders
                .iter()
                .find(|o| o.id == order_id)
                .unwrap()
                .status,
            PaperOrderStatus::Canceled
        );
        assert!(matches!(
            engine.cancel(order_id),
            Err(PaperTradingError::InvalidTransition)
        ));
    }

    #[test]
    fn market_order_without_a_quote_is_rejected() {
        let (mut engine, account) = engine_with_account();
        assert!(matches!(
            engine.submit(request(account, PaperOrderType::Market, 1.0)),
            Err(PaperTradingError::NoMarketData)
        ));
    }

    // MARK: Funds and protection

    fn find(engine: &PaperTradingEngine, id: u64) -> &PaperOrder {
        engine
            .snapshot()
            .orders
            .iter()
            .find(|order| order.id == id)
            .unwrap()
    }

    fn sell_limit(account_id: u64, quantity: f64) -> PaperOrderRequest {
        PaperOrderRequest {
            side: PaperOrderSide::Sell,
            limit_price: Some(200.0),
            ..request(account_id, PaperOrderType::Limit, quantity)
        }
    }

    fn bracketed_buy(account_id: u64, quantity: f64) -> PaperOrderRequest {
        PaperOrderRequest {
            take_profit: Some(120.0),
            stop_loss: Some(50.0),
            ..request(account_id, PaperOrderType::Market, quantity)
        }
    }

    fn working_protection(engine: &PaperTradingEngine) -> Vec<&PaperOrder> {
        engine
            .snapshot()
            .orders
            .iter()
            .filter(|order| order.role != PaperOrderRole::Entry && order.status.is_working())
            .collect()
    }

    fn small_engine(balance: f64, settings: PaperAccountSettings) -> (PaperTradingEngine, u64) {
        let mut engine = PaperTradingEngine::new(PaperTradingSnapshot::default());
        let id = engine
            .create_account("Small", PaperCurrency::Usd, balance, settings)
            .unwrap();
        (engine, id)
    }

    #[test]
    fn a_modification_cannot_grow_an_order_past_the_funds() {
        let (mut engine, account) = engine_with_account();
        engine.process(quote("Binance:BTCUSDT", 110.0)).unwrap();
        let id = engine
            .submit(request(account, PaperOrderType::Limit, 1.0))
            .unwrap();
        let result = engine.modify(
            id,
            PaperOrderChanges {
                quantity: Some(2_000.0),
                ..PaperOrderChanges::default()
            },
        );
        assert!(matches!(
            result,
            Err(PaperTradingError::InsufficientFunds { .. })
        ));
        assert_eq!(find(&engine, id).original_quantity, 1.0);
        assert_eq!(find(&engine, id).reserved_margin, 100.0);
        // A modification that still fits goes through, and is not charged against itself.
        engine
            .modify(
                id,
                PaperOrderChanges {
                    quantity: Some(1_000.0),
                    ..PaperOrderChanges::default()
                },
            )
            .unwrap();
        assert_eq!(find(&engine, id).reserved_margin, 100_000.0);
    }

    #[test]
    fn exits_beyond_the_position_reserve_margin_for_the_short_they_could_open() {
        let (mut engine, account) = small_engine(1_000.0, PaperAccountSettings::default());
        engine.process(quote("Binance:BTCUSDT", 100.0)).unwrap();
        engine
            .submit(request(account, PaperOrderType::Market, 5.0))
            .unwrap();
        let first = engine.submit(sell_limit(account, 5.0)).unwrap();
        assert_eq!(find(&engine, first).reserved_margin, 0.0);
        // The position is already spoken for by the first exit, so a second one opens a short.
        let second = engine.submit(sell_limit(account, 5.0));
        assert!(matches!(
            second,
            Err(PaperTradingError::InsufficientFunds { .. })
        ));
    }

    #[test]
    fn overlapping_exits_on_a_funded_account_each_reserve_what_they_could_open() {
        let (mut engine, account) = engine_with_account();
        engine.process(quote("Binance:BTCUSDT", 100.0)).unwrap();
        engine
            .submit(request(account, PaperOrderType::Market, 1.0))
            .unwrap();
        let first = engine.submit(sell_limit(account, 1.0)).unwrap();
        let second = engine.submit(sell_limit(account, 1.0)).unwrap();
        let third = engine.submit(sell_limit(account, 1.0)).unwrap();
        assert_eq!(find(&engine, first).reserved_margin, 0.0);
        assert_eq!(find(&engine, second).reserved_margin, 200.0);
        assert_eq!(find(&engine, third).reserved_margin, 200.0);
    }

    #[test]
    fn a_fill_that_gaps_past_the_reserved_price_is_refused() {
        let (mut engine, account) = small_engine(1_000.0, PaperAccountSettings::default());
        engine.process(quote("Binance:BTCUSDT", 100.0)).unwrap();
        let id = engine
            .submit(PaperOrderRequest {
                stop_price: Some(101.0),
                ..request(account, PaperOrderType::Stop, 9.0)
            })
            .unwrap();
        assert_eq!(find(&engine, id).status, PaperOrderStatus::Working);

        // The stop reserved 9 x 101; the market gaps to 200.5 and the fill would cost twice that.
        engine.process(quote("Binance:BTCUSDT", 200.0)).unwrap();
        let order = find(&engine, id);
        assert_eq!(order.status, PaperOrderStatus::Rejected);
        assert!(order.rejection_reason.is_some());
        assert!(engine.snapshot().positions.is_empty());
        assert!(
            engine
                .snapshot()
                .order_events
                .iter()
                .any(|event| event.order_id == id && event.kind == PaperOrderEventKind::Rejected)
        );
    }

    #[test]
    fn commission_counts_toward_the_funds_an_opening_order_needs() {
        let settings = PaperAccountSettings {
            commission: PaperCommissionConfiguration::FixedPerOrder { value: 50.0 },
            ..PaperAccountSettings::default()
        };
        let (mut engine, account) = small_engine(1_000.0, settings);
        engine.process(quote("Binance:BTCUSDT", 100.0)).unwrap();
        // 9.9 x 100.5 = 994.95 fits 1,000 alone; with the 50 fee it does not.
        let result = engine.submit(request(account, PaperOrderType::Market, 9.9));
        assert!(matches!(
            result,
            Err(PaperTradingError::InsufficientFunds { .. })
        ));
    }

    #[test]
    fn closing_is_never_refused_for_its_commission() {
        let settings = PaperAccountSettings {
            commission: PaperCommissionConfiguration::FixedPerOrder { value: 50.0 },
            ..PaperAccountSettings::default()
        };
        let (mut engine, account) = small_engine(1_000.0, settings);
        engine.process(quote("Binance:BTCUSDT", 100.0)).unwrap();
        engine
            .submit(request(account, PaperOrderType::Market, 9.0))
            .unwrap();
        engine
            .submit(PaperOrderRequest {
                side: PaperOrderSide::Sell,
                ..request(account, PaperOrderType::Market, 9.0)
            })
            .unwrap();
        assert!(engine.snapshot().positions.is_empty());
    }

    #[test]
    fn reversing_a_position_cancels_the_protection_that_would_now_add_to_it() {
        let (mut engine, account) = engine_with_account();
        engine.process(quote("Binance:BTCUSDT", 100.0)).unwrap();
        engine.submit(bracketed_buy(account, 1.0)).unwrap();
        assert_eq!(working_protection(&engine).len(), 2);

        engine
            .submit(PaperOrderRequest {
                side: PaperOrderSide::Sell,
                ..request(account, PaperOrderType::Market, 2.0)
            })
            .unwrap();
        assert_eq!(engine.snapshot().positions[0].signed_quantity, -1.0);
        assert!(working_protection(&engine).is_empty());
        let canceled = engine
            .snapshot()
            .order_events
            .iter()
            .filter(|event| {
                event.kind == PaperOrderEventKind::Canceled && event.message.contains("reversed")
            })
            .count();
        assert_eq!(canceled, 2, "both exits are audited");
    }

    #[test]
    fn a_take_profit_fill_audits_the_cancel_of_its_stop() {
        let (mut engine, account) = engine_with_account();
        engine.process(quote("Binance:BTCUSDT", 100.0)).unwrap();
        engine.submit(bracketed_buy(account, 1.0)).unwrap();
        let stop = engine
            .snapshot()
            .orders
            .iter()
            .find(|order| order.role == PaperOrderRole::StopLoss)
            .unwrap()
            .id;

        engine.process(quote("Binance:BTCUSDT", 125.0)).unwrap();
        assert!(engine.snapshot().positions.is_empty());
        assert_eq!(find(&engine, stop).status, PaperOrderStatus::Canceled);
        assert!(
            engine
                .snapshot()
                .order_events
                .iter()
                .any(|event| event.order_id == stop && event.kind == PaperOrderEventKind::Canceled)
        );
    }

    #[test]
    fn a_partial_close_shrinks_protection_to_what_is_left() {
        let (mut engine, account) = engine_with_account();
        engine.process(quote("Binance:BTCUSDT", 100.0)).unwrap();
        engine.submit(bracketed_buy(account, 2.0)).unwrap();
        engine
            .submit(PaperOrderRequest {
                side: PaperOrderSide::Sell,
                ..request(account, PaperOrderType::Market, 1.0)
            })
            .unwrap();
        let protection = working_protection(&engine);
        assert_eq!(protection.len(), 2);
        assert!(
            protection
                .iter()
                .all(|order| order.original_quantity == 1.0)
        );
    }

    #[test]
    fn an_entry_that_only_reduces_arms_no_protection() {
        let (mut engine, account) = engine_with_account();
        engine.process(quote("Binance:BTCUSDT", 100.0)).unwrap();
        engine
            .submit(request(account, PaperOrderType::Market, 1.0))
            .unwrap();
        engine
            .submit(PaperOrderRequest {
                side: PaperOrderSide::Sell,
                take_profit: Some(90.0),
                stop_loss: Some(150.0),
                ..request(account, PaperOrderType::Market, 1.0)
            })
            .unwrap();
        assert!(engine.snapshot().positions.is_empty());
        assert!(working_protection(&engine).is_empty());
    }
}
