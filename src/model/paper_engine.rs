#![allow(dead_code)]
//! Paper-trading execution engine: the order/fill/position state machine over
//! `PaperTradingSnapshot`. Ported from `PaperTradingEngine.swift`. Quotes arrive
//! via `process`; the engine itself never touches the network.

use super::paper::{
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
        order.reserved_margin = self.required_margin(order, account)?;
        let available = self.metrics(account.id).available_funds;
        if order.reserved_margin > available {
            return Err(PaperTradingError::InsufficientFunds {
                required: order.reserved_margin,
                available,
            });
        }
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
        order.reserved_margin = self.required_margin(&order, &account)?;
        self.replace(order.clone());
        self.event(
            order.clone(),
            PaperOrderEventKind::Modified,
            "PAPER order modified".to_owned(),
        );
        self.evaluate(order.id, None)
    }

    /// Feed a live quote. Only quotes that could have moved an order are persisted.
    pub fn process(&mut self, quote: PaperQuote) -> Result<(), PaperTradingError> {
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
                .filter(|order| order.instrument.key == key && order.status.is_working())
                .collect();
            working.sort_by_key(|order| order.created_at);
            working.iter().map(|order| order.id).collect()
        };
        for id in ids {
            self.evaluate(id, None)?;
        }
        Ok(())
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
        let remaining = position.signed_quantity.abs();
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
        self.resize_protective_orders(fill.account_id, &fill.instrument.key, remaining);
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
        let group = self.mint();
        let exit_side = if parent.side == PaperOrderSide::Buy {
            PaperOrderSide::Sell
        } else {
            PaperOrderSide::Buy
        };
        if let Some(price) = take_profit {
            self.append_bracket_child(
                parent,
                exit_side,
                group,
                PaperOrderType::Limit,
                price,
                PaperOrderRole::TakeProfit,
            );
        }
        if let Some(price) = stop_loss {
            self.append_bracket_child(
                parent,
                exit_side,
                group,
                PaperOrderType::Stop,
                price,
                PaperOrderRole::StopLoss,
            );
        }
    }

    fn append_bracket_child(
        &mut self,
        parent: &PaperOrder,
        side: PaperOrderSide,
        group: u64,
        kind: PaperOrderType,
        price: f64,
        role: PaperOrderRole,
    ) {
        let child = PaperOrder {
            id: self.mint(),
            account_id: parent.account_id,
            instrument: parent.instrument.clone(),
            side,
            kind,
            original_quantity: parent.filled_quantity,
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

    fn resize_protective_orders(&mut self, account_id: u64, instrument_key: &str, remaining: f64) {
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
            if remaining == 0.0 {
                self.snapshot.orders[index].status = PaperOrderStatus::Canceled;
            } else {
                let filled = self.snapshot.orders[index].filled_quantity;
                self.snapshot.orders[index].original_quantity = filled + remaining;
            }
            self.snapshot.orders[index].updated_at = now_ms();
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

    fn required_margin(
        &self,
        order: &PaperOrder,
        account: &PaperAccount,
    ) -> Result<f64, PaperTradingError> {
        if order.role != PaperOrderRole::Entry {
            return Ok(0.0);
        }
        let price: f64;
        if matches!(
            order.kind,
            PaperOrderType::Limit | PaperOrderType::StopLimit
        ) && let Some(limit) = order.limit_price
        {
            price = limit;
        } else if order.kind == PaperOrderType::Stop
            && let Some(stop) = order.stop_price
        {
            price = stop;
        } else if let Some((quote, _)) = self
            .snapshot
            .quotes
            .get(&order.instrument.key)
            .and_then(|quote| quote.executable_price(order.side))
        {
            price = quote;
        } else {
            return Err(PaperTradingError::NoMarketData);
        }
        let signed_position = self
            .snapshot
            .positions
            .iter()
            .find(|position| {
                position.account_id == order.account_id
                    && position.instrument.key == order.instrument.key
            })
            .map_or(0.0, |position| position.signed_quantity);
        let reduces_existing = (signed_position > 0.0 && order.side == PaperOrderSide::Sell)
            || (signed_position < 0.0 && order.side == PaperOrderSide::Buy);
        let reducible_quantity = if reduces_existing {
            signed_position.abs()
        } else {
            0.0
        };
        let exposure_increasing = (order.remaining_quantity() - reducible_quantity).max(0.0);
        let leverage = account
            .settings
            .leverage
            .leverage(order.instrument.asset_class)
            .max(1.0);
        Ok(exposure_increasing * price * order.instrument.contract_multiplier / leverage)
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
    if !crate::model::paper_ticket::is_aligned(value, increment) {
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
}
