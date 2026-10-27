//! Backtest broker for `strategy()` scripts, ported from `PineBrokerEmulator*.swift`. A plain
//! value type the runtime keeps inside its per-bar state.

use super::strategy::{OpenTrade, StrategyReport, StrategySettings, Trade};
use crate::model::Kline;

pub const EPSILON: f64 = 1e-12;
const MAX_FILLS_PER_SEGMENT: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OrderKind {
    Entry,
    Order,
    Exit,
    Close,
    CloseAll,
}

impl OrderKind {
    pub fn is_entry_like(self) -> bool {
        matches!(self, Self::Entry | Self::Order)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Order {
    pub kind: OrderKind,
    pub id: String,
    pub is_long: bool,
    pub quantity: Option<f64>,
    pub quantity_percent: Option<f64>,
    pub limit: Option<f64>,
    pub stop: Option<f64>,
    /// `strategy.exit`: the entry this exit protects; None protects every open trade.
    pub from_entry: Option<String>,
    pub profit_ticks: Option<f64>,
    pub loss_ticks: Option<f64>,
    pub sequence: usize,
    /// `comment`: shown with the order's trade; an exit's `comment_profit` and `comment_loss` take
    /// its place when the profit or the loss level fills it.
    pub comment: String,
    pub comment_profit: Option<String>,
    pub comment_loss: Option<String>,
}

impl Order {
    /// Fills at the next opportunity price instead of waiting for a level.
    pub fn is_market(&self) -> bool {
        match self.kind {
            OrderKind::Entry | OrderKind::Order => self.limit.is_none() && self.stop.is_none(),
            OrderKind::Close | OrderKind::CloseAll => true,
            OrderKind::Exit => false,
        }
    }

    /// Whether this exit order covers the trade opened by `entry_id`.
    pub fn protects(&self, entry_id: &str) -> bool {
        self.from_entry.as_deref().is_none_or(|id| id == entry_id)
    }
}

/// The bar an order is being processed against.
#[derive(Clone, Debug)]
pub struct BarContext {
    pub bar: Kline,
    pub bar_index: usize,
    pub mintick: f64,
}

#[derive(Clone, Debug)]
pub enum TriggerAction {
    FillEntry(Order, bool),
    /// A stop-limit entry's stop was crossed; it becomes a plain limit order.
    Activate(Order),
    Exit(Order, usize, bool),
}

#[derive(Clone, Debug)]
pub struct Trigger {
    pub action: TriggerAction,
    pub price: f64,
    pub distance: f64,
    pub sequence: usize,
}

#[derive(Clone, Debug)]
pub struct Broker {
    pub settings: StrategySettings,
    pub open_trades: Vec<OpenTrade>,
    pub closed_trades: Vec<Trade>,
    pub net_profit: f64,
    pub orders: Vec<Order>,
    equity_curve: Vec<f64>,
    extremes: super::strategy::EquityExtremes,
    last_close: f64,
    next_sequence: usize,
}

impl Broker {
    pub fn new(settings: StrategySettings) -> Self {
        let extremes = super::strategy::EquityExtremes::starting_at(settings.initial_capital);
        Self {
            settings,
            open_trades: Vec::new(),
            closed_trades: Vec::new(),
            net_profit: 0.0,
            orders: Vec::new(),
            equity_curve: Vec::new(),
            extremes,
            last_close: 0.0,
            next_sequence: 0,
        }
    }

    // MARK: Position and equity

    /// Signed contracts: positive long, negative short.
    pub fn position_size(&self) -> f64 {
        self.open_trades
            .iter()
            .map(|trade| {
                if trade.is_long {
                    trade.quantity
                } else {
                    -trade.quantity
                }
            })
            .sum()
    }

    pub fn average_price(&self) -> f64 {
        let total: f64 = self.open_trades.iter().map(|trade| trade.quantity).sum();
        if total <= 0.0 {
            return 0.0;
        }
        self.open_trades
            .iter()
            .map(|trade| trade.entry_price * trade.quantity)
            .sum::<f64>()
            / total
    }

    fn open_entry_commission(&self) -> f64 {
        self.open_trades
            .iter()
            .map(|trade| trade.entry_commission)
            .sum()
    }

    /// Gross unrealized P&L of the open trades at `price`.
    pub fn open_profit(&self, price: f64) -> f64 {
        self.open_trades
            .iter()
            .map(|trade| {
                let direction = if trade.is_long { 1.0 } else { -1.0 };
                direction * (price - trade.entry_price) * trade.quantity
            })
            .sum()
    }

    pub fn equity(&self, price: f64) -> f64 {
        self.settings.initial_capital + self.net_profit + self.open_profit(price)
            - self.open_entry_commission()
    }

    pub fn record_equity(&mut self, close: f64) {
        self.last_close = close;
        let equity = self.equity(close);
        self.equity_curve.push(equity);
        self.extremes.include(equity);
    }

    /// The equity curve's extremes up to the bar being run, whose equity at `price` is not recorded
    /// until the bar ends.
    pub fn equity_extremes(&self, price: f64) -> super::strategy::EquityExtremes {
        let mut extremes = self.extremes;
        extremes.include(self.equity(price));
        extremes
    }

    pub fn report(&self) -> StrategyReport {
        StrategyReport {
            settings: self.settings.clone(),
            trades: self.closed_trades.clone(),
            open_trades: self.open_trades.clone(),
            equity: self.equity_curve.clone(),
            open_profit: self.open_profit(self.last_close) - self.open_entry_commission(),
            chart: crate::model::ChartKind::Standard,
        }
    }

    // MARK: Order book

    /// Placing an order with the id of a pending one of the same family replaces it.
    pub fn place(&mut self, mut order: Order) {
        order.sequence = self.next_sequence;
        self.next_sequence += 1;
        match order.kind {
            OrderKind::Entry | OrderKind::Order => {
                self.orders
                    .retain(|o| o.kind.is_entry_like() && o.id != order.id);
            }
            OrderKind::Exit => self
                .orders
                .retain(|o| o.kind == OrderKind::Exit && o.id != order.id),
            OrderKind::Close => self
                .orders
                .retain(|o| o.kind == OrderKind::Close && o.id != order.id),
            OrderKind::CloseAll => self.orders.retain(|o| o.kind != OrderKind::CloseAll),
        }
        // An exit with no level at all cancels the previous one.
        if order.kind == OrderKind::Exit
            && order.stop.is_none()
            && order.limit.is_none()
            && order.profit_ticks.is_none()
            && order.loss_ticks.is_none()
        {
            return;
        }
        self.orders.push(order);
    }

    pub fn cancel(&mut self, id: &str) {
        self.orders.retain(|order| order.id != id);
    }

    pub fn cancel_all(&mut self) {
        self.orders.clear();
    }

    // MARK: Filling

    /// Lets every open trade see the highest and lowest prices of `bar`.
    fn touch_bar(&mut self, bar: &Kline) {
        self.touch(bar.high);
        self.touch(bar.low);
    }

    /// Records that the market reached `price` for every open trade, for its run-up and drawdown.
    fn touch(&mut self, price: f64) {
        for trade in &mut self.open_trades {
            trade.highest_price = trade.highest_price.max(price);
            trade.lowest_price = trade.lowest_price.min(price);
        }
    }

    pub fn process(&mut self, bar: Kline, bar_index: usize, mintick: f64) {
        if self.orders.is_empty() {
            self.touch_bar(&bar);
            return;
        }
        let open = bar.open;
        let context = BarContext {
            bar,
            bar_index,
            mintick,
        };
        self.touch(open);
        self.fill_market_orders(open, &context);
        let path = Self::price_path(&context.bar);
        for index in 0..path.len().saturating_sub(1) {
            let mut start = path[index];
            let end = path[index + 1];
            let mut fills = 0;
            while fills < MAX_FILLS_PER_SEGMENT {
                let Some(trigger) = self.next_trigger(start, end, mintick) else {
                    break;
                };
                fills += 1;
                let trigger_price = trigger.price;
                // The trades open now have been this far; one this trigger opens starts here.
                self.touch(trigger_price);
                self.apply(trigger, &context);
                start = trigger_price;
            }
            self.touch(end);
        }
        self.cull_exits();
    }

    /// TradingView's intrabar assumption: price goes to the nearer extreme first.
    fn price_path(bar: &Kline) -> [f64; 4] {
        let toward_high_first = (bar.open - bar.high).abs() < (bar.open - bar.low).abs();
        if toward_high_first {
            [bar.open, bar.high, bar.low, bar.close]
        } else {
            [bar.open, bar.low, bar.high, bar.close]
        }
    }

    pub fn fill_market_orders_at_close(&mut self, bar: Kline, bar_index: usize, mintick: f64) {
        let close = bar.close;
        let context = BarContext {
            bar,
            bar_index,
            mintick,
        };
        self.fill_market_orders(close, &context);
    }

    fn fill_market_orders(&mut self, price: f64, context: &BarContext) {
        let mut market: Vec<Order> = self
            .orders
            .iter()
            .filter(|order| order.is_market())
            .cloned()
            .collect();
        market.sort_by_key(|order| order.sequence);
        if market.is_empty() {
            return;
        }
        self.orders.retain(|order| !order.is_market());
        for order in market {
            match order.kind {
                OrderKind::Entry | OrderKind::Order => {
                    self.execute_entry(order, price, true, context);
                }
                OrderKind::Close => self.execute_close(order, price, context),
                OrderKind::CloseAll => self.execute_close_all(order, price, context),
                OrderKind::Exit => {}
            }
        }
        self.cull_exits();
    }

    fn cull_exits(&mut self) {
        let pending: Vec<Order> = self
            .orders
            .iter()
            .filter(|order| order.kind.is_entry_like())
            .cloned()
            .collect();
        let trades = self.open_trades.clone();
        self.orders.retain(|order| {
            if order.kind != OrderKind::Exit {
                return true;
            }
            if trades.iter().any(|trade| order.protects(&trade.entry_id)) {
                return true;
            }
            pending.iter().any(|entry| order.protects(&entry.id))
        });
    }

    // MARK: Sizing and commission

    fn slipped(&self, price: f64, buying: bool, mintick: f64) -> f64 {
        let amount = self.settings.slippage as f64 * mintick;
        if buying {
            price + amount
        } else {
            price - amount
        }
    }

    fn default_quantity(&self, price: f64) -> f64 {
        if price <= 0.0 {
            return 0.0;
        }
        match self.settings.quantity_type {
            super::strategy::QuantityType::Fixed => self.settings.quantity_value,
            super::strategy::QuantityType::Cash => self.settings.quantity_value / price,
            super::strategy::QuantityType::PercentOfEquity => {
                self.equity(price).max(0.0) * self.settings.quantity_value / 100.0 / price
            }
        }
    }

    fn commission(&self, quantity: f64, price: f64, charges_order: bool) -> f64 {
        match self.settings.commission_type {
            super::strategy::CommissionType::Percent => {
                quantity * price * self.settings.commission_value / 100.0
            }
            super::strategy::CommissionType::CashPerContract => {
                quantity * self.settings.commission_value
            }
            super::strategy::CommissionType::CashPerOrder => {
                if charges_order {
                    self.settings.commission_value
                } else {
                    0.0
                }
            }
        }
    }

    // MARK: Orders that open

    fn execute_entry(
        &mut self,
        order: Order,
        raw_price: f64,
        use_slippage: bool,
        context: &BarContext,
    ) {
        let price = if use_slippage {
            self.slipped(raw_price, order.is_long, context.mintick)
        } else {
            raw_price
        };
        let position = self.position_size();
        let opposes_position = if order.is_long {
            position < -EPSILON
        } else {
            position > EPSILON
        };

        match order.kind {
            OrderKind::Entry => {
                if opposes_position {
                    self.close(
                        &|trade| trade.is_long != order.is_long,
                        None,
                        price,
                        &order.id,
                        &order.comment,
                        context,
                    );
                } else {
                    let same_side = self
                        .open_trades
                        .iter()
                        .filter(|trade| trade.is_long == order.is_long)
                        .count();
                    if same_side >= self.settings.pyramiding.max(1) {
                        return;
                    }
                }
                let quantity = order
                    .quantity
                    .unwrap_or_else(|| self.default_quantity(price));
                self.open(&order, quantity, price, context);
            }
            OrderKind::Order => {
                let mut remaining = order
                    .quantity
                    .unwrap_or_else(|| self.default_quantity(price));
                if opposes_position {
                    let reduce = remaining.min(position.abs());
                    self.close(
                        &|trade| trade.is_long != order.is_long,
                        Some(reduce),
                        price,
                        &order.id,
                        &order.comment,
                        context,
                    );
                    remaining -= reduce;
                }
                self.open(&order, remaining, price, context);
            }
            OrderKind::Exit | OrderKind::Close | OrderKind::CloseAll => {}
        }
    }

    fn open(&mut self, order: &Order, quantity: f64, price: f64, context: &BarContext) {
        if !quantity.is_finite() || quantity <= EPSILON {
            return;
        }
        self.open_trades.push(OpenTrade {
            entry_id: order.id.clone(),
            entry_comment: order.comment.clone(),
            highest_price: price,
            lowest_price: price,
            is_long: order.is_long,
            quantity,
            entry_bar: context.bar_index,
            entry_time: context.bar.open_time,
            entry_price: price,
            entry_commission: self.commission(quantity, price, true),
        });
    }

    // MARK: Orders that close

    fn execute_close(&mut self, order: Order, price: f64, context: &BarContext) {
        let matching: Vec<&OpenTrade> = self
            .open_trades
            .iter()
            .filter(|trade| trade.entry_id == order.id)
            .collect();
        let total: f64 = matching.iter().map(|trade| trade.quantity).sum();
        let quantity = order
            .quantity
            .or_else(|| order.quantity_percent.map(|p| total * p / 100.0));
        let is_long = matching.first().is_none_or(|trade| trade.is_long);
        self.close(
            &|trade| trade.entry_id == order.id,
            quantity,
            self.slipped(price, !is_long, context.mintick),
            &format!("Close entry(s) order {}", order.id),
            &order.comment,
            context,
        );
    }

    fn execute_close_all(&mut self, order: Order, price: f64, context: &BarContext) {
        let is_long = self.position_size() > 0.0;
        self.close(
            &|_| true,
            None,
            self.slipped(price, !is_long, context.mintick),
            &order.id,
            &order.comment,
            context,
        );
    }

    /// Closes up to `quantity` (None = everything) of the open trades `selects` chooses.
    fn close(
        &mut self,
        selects: &impl Fn(&OpenTrade) -> bool,
        quantity: Option<f64>,
        price: f64,
        exit_id: &str,
        exit_comment: &str,
        context: &BarContext,
    ) {
        let mut remaining = quantity.unwrap_or(f64::INFINITY);
        let mut charged_order = false;
        let mut index = 0;
        while index < self.open_trades.len() && remaining > EPSILON {
            let mut trade = self.open_trades[index].clone();
            if !selects(&trade) {
                index += 1;
                continue;
            }
            // The price it closes at is one the trade has seen.
            trade.highest_price = trade.highest_price.max(price);
            trade.lowest_price = trade.lowest_price.min(price);
            let closing = trade.quantity.min(remaining);
            let entry_share = trade.entry_commission * (closing / trade.quantity);
            let exit_commission = self.commission(closing, price, !charged_order);
            charged_order = true;
            let direction = if trade.is_long { 1.0 } else { -1.0 };
            let profit =
                (price - trade.entry_price) * direction * closing - entry_share - exit_commission;
            let (max_runup, max_drawdown) = super::strategy::excursions(
                trade.is_long,
                trade.entry_price,
                closing,
                trade.highest_price,
                trade.lowest_price,
            );
            self.closed_trades.push(Trade {
                id: self.closed_trades.len() + 1,
                entry_id: trade.entry_id.clone(),
                entry_comment: trade.entry_comment.clone(),
                exit_comment: exit_comment.to_owned(),
                max_runup,
                max_drawdown,
                exit_id: exit_id.to_owned(),
                is_long: trade.is_long,
                quantity: closing,
                entry_bar: trade.entry_bar,
                entry_time: trade.entry_time,
                entry_price: trade.entry_price,
                exit_bar: context.bar_index,
                exit_time: context.bar.open_time,
                exit_price: price,
                commission: entry_share + exit_commission,
                profit,
            });
            self.net_profit += profit;
            remaining -= closing;
            if closing >= trade.quantity - EPSILON {
                self.open_trades.remove(index);
            } else {
                self.open_trades[index].quantity -= closing;
                self.open_trades[index].entry_commission -= entry_share;
                index += 1;
            }
        }
    }

    // MARK: Triggers

    pub fn next_trigger(&self, from: f64, to: f64, mintick: f64) -> Option<Trigger> {
        let mut best: Option<Trigger> = None;
        for order in &self.orders {
            match order.kind {
                OrderKind::Entry | OrderKind::Order => {
                    let candidate = entry_trigger(order, from, to);
                    if let Some(candidate) = candidate {
                        consider(&mut best, candidate);
                    }
                }
                OrderKind::Exit => {
                    for (index, trade) in self.open_trades.iter().enumerate() {
                        if !order.protects(&trade.entry_id) {
                            continue;
                        }
                        let (stop, limit) = exit_levels(order, trade, mintick);
                        if let Some(stop) = stop
                            && let Some(hit) = reach(stop, !trade.is_long, from, to)
                        {
                            consider(
                                &mut best,
                                Trigger {
                                    action: TriggerAction::Exit(order.clone(), index, true),
                                    price: hit.0,
                                    distance: hit.1,
                                    sequence: order.sequence,
                                },
                            );
                        }
                        if let Some(limit) = limit
                            && let Some(hit) = reach(limit, trade.is_long, from, to)
                        {
                            consider(
                                &mut best,
                                Trigger {
                                    action: TriggerAction::Exit(order.clone(), index, false),
                                    price: hit.0,
                                    distance: hit.1,
                                    sequence: order.sequence,
                                },
                            );
                        }
                    }
                }
                OrderKind::Close | OrderKind::CloseAll => {}
            }
        }
        best
    }

    fn apply(&mut self, trigger: Trigger, context: &BarContext) {
        match trigger.action {
            TriggerAction::Activate(order) => {
                if let Some(index) = self
                    .orders
                    .iter()
                    .position(|o| o.sequence == order.sequence)
                {
                    self.orders[index].stop = None;
                }
            }
            TriggerAction::FillEntry(order, via_stop) => {
                self.orders.retain(|o| o.sequence != order.sequence);
                self.execute_entry(order, trigger.price, via_stop, context);
            }
            TriggerAction::Exit(order, trade_index, via_stop) => {
                self.apply_exit(order, trade_index, via_stop, trigger.price, context);
            }
        }
    }

    fn apply_exit(
        &mut self,
        order: Order,
        trade_index: usize,
        via_stop: bool,
        trigger_price: f64,
        context: &BarContext,
    ) {
        if trade_index >= self.open_trades.len() {
            return;
        }
        let trade = self.open_trades[trade_index].clone();
        let requested = order
            .quantity
            .or_else(|| order.quantity_percent.map(|p| trade.quantity * p / 100.0));
        let price = if via_stop {
            self.slipped(trigger_price, !trade.is_long, context.mintick)
        } else {
            trigger_price
        };
        // The level that filled the order picks the comment: the loss level is the stop.
        let specific = if via_stop {
            order.comment_loss.as_ref()
        } else {
            order.comment_profit.as_ref()
        };
        let comment = specific.unwrap_or(&order.comment).clone();
        self.close(
            &|t| t.entry_id == trade.entry_id && t.entry_bar == trade.entry_bar,
            Some(requested.unwrap_or(trade.quantity).min(trade.quantity)),
            price,
            &order.id,
            &comment,
            context,
        );
        if !self.open_trades.iter().any(|t| order.protects(&t.entry_id)) {
            self.orders.retain(|o| o.sequence != order.sequence);
        }
    }
}

impl Default for Broker {
    fn default() -> Self {
        Self::new(StrategySettings::default())
    }
}

/// The earliest trigger wins; ties go to the older order.
fn consider(best: &mut Option<Trigger>, candidate: Trigger) {
    if best.as_ref().is_some_and(|current| {
        (current.distance, current.sequence) <= (candidate.distance, candidate.sequence)
    }) {
        return;
    }
    *best = Some(candidate);
}

fn reach(level: f64, at_or_above: bool, p0: f64, p1: f64) -> Option<(f64, f64)> {
    if at_or_above {
        if p0 >= level {
            return Some((p0, 0.0));
        }
        if p1 >= level {
            return Some((level, level - p0));
        }
    } else {
        if p0 <= level {
            return Some((p0, 0.0));
        }
        if p1 <= level {
            return Some((level, p0 - level));
        }
    }
    None
}

fn entry_trigger(order: &Order, from: f64, to: f64) -> Option<Trigger> {
    if let Some(stop) = order.stop
        && let Some(hit) = reach(stop, order.is_long, from, to)
    {
        let action = if order.limit.is_none() {
            TriggerAction::FillEntry(order.clone(), true)
        } else {
            TriggerAction::Activate(order.clone())
        };
        return Some(Trigger {
            action,
            price: hit.0,
            distance: hit.1,
            sequence: order.sequence,
        });
    }
    if order.stop.is_none()
        && let Some(limit) = order.limit
        && let Some(hit) = reach(limit, !order.is_long, from, to)
    {
        return Some(Trigger {
            action: TriggerAction::FillEntry(order.clone(), false),
            price: hit.0,
            distance: hit.1,
            sequence: order.sequence,
        });
    }
    None
}

fn exit_levels(order: &Order, trade: &OpenTrade, mintick: f64) -> (Option<f64>, Option<f64>) {
    let direction = if trade.is_long { 1.0 } else { -1.0 };
    let limit = order.limit.or_else(|| {
        order
            .profit_ticks
            .map(|ticks| trade.entry_price + direction * ticks * mintick)
    });
    let stop = order.stop.or_else(|| {
        order
            .loss_ticks
            .map(|ticks| trade.entry_price - direction * ticks * mintick)
    });
    (stop, limit)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar(open: f64, high: f64, low: f64, close: f64) -> Kline {
        Kline {
            open_time: 0,
            close_time: 0,
            open,
            high,
            low,
            close,
            volume: 0.0,
        }
    }

    fn order(kind: OrderKind, id: &str, is_long: bool, quantity: Option<f64>) -> Order {
        Order {
            kind,
            id: id.to_owned(),
            is_long,
            quantity,
            quantity_percent: None,
            limit: None,
            stop: None,
            from_entry: None,
            profit_ticks: None,
            loss_ticks: None,
            sequence: 0,
            comment: String::new(),
            comment_profit: None,
            comment_loss: None,
        }
    }

    fn settings() -> StrategySettings {
        StrategySettings {
            commission_value: 0.0,
            slippage: 0,
            ..StrategySettings::default()
        }
    }

    #[test]
    fn entry_fills_at_open_and_close_realizes_profit() {
        let mut broker = Broker::new(settings());
        broker.place(order(OrderKind::Entry, "long1", true, Some(2.0)));
        broker.process(bar(100.0, 110.0, 90.0, 105.0), 0, 0.01);
        assert_eq!(broker.position_size(), 2.0);
        assert_eq!(broker.open_trades[0].entry_price, 100.0);

        broker.place(order(OrderKind::CloseAll, "close", true, None));
        broker.process(bar(120.0, 130.0, 110.0, 125.0), 1, 0.01);
        assert!(broker.open_trades.is_empty());
        assert_eq!(broker.closed_trades.len(), 1);
        assert!((broker.closed_trades[0].profit - 40.0).abs() < 1e-9);
        assert!((broker.net_profit - 40.0).abs() < 1e-9);
    }

    #[test]
    fn a_short_gains_when_price_falls() {
        let mut broker = Broker::new(settings());
        broker.place(order(OrderKind::Entry, "short1", false, Some(1.0)));
        broker.process(bar(100.0, 110.0, 90.0, 95.0), 0, 0.01);
        assert_eq!(broker.position_size(), -1.0);

        broker.place(order(OrderKind::CloseAll, "close", false, None));
        broker.process(bar(80.0, 85.0, 75.0, 78.0), 1, 0.01);
        assert!((broker.net_profit - 20.0).abs() < 1e-9); // (100 - 80) * 1
    }

    #[test]
    fn a_limit_entry_waits_for_the_price_path() {
        let mut broker = Broker::new(settings());
        let mut entry = order(OrderKind::Entry, "limit1", true, Some(1.0));
        entry.limit = Some(90.0);
        broker.place(entry);
        broker.process(bar(100.0, 110.0, 95.0, 105.0), 0, 0.01);
        assert_eq!(broker.position_size(), 0.0);
        broker.process(bar(95.0, 96.0, 88.0, 94.0), 1, 0.01);
        assert_eq!(broker.position_size(), 1.0);
        assert_eq!(broker.open_trades[0].entry_price, 90.0);
    }

    #[test]
    fn a_trades_run_up_and_drawdown_follow_the_prices_it_saw() {
        let mut broker = Broker::new(settings());
        broker.place(order(OrderKind::Entry, "L", true, Some(2.0)));
        // Opened at the open of a bar that ranged 90 to 110.
        broker.process(bar(100.0, 110.0, 90.0, 105.0), 0, 0.01);
        assert_eq!(broker.open_trades[0].excursions(), (20.0, 20.0));
        // A bar with no orders pending still widens them: this one reached 120 and 85.
        broker.process(bar(105.0, 120.0, 85.0, 118.0), 1, 0.01);
        assert_eq!(broker.open_trades[0].excursions(), (40.0, 30.0));
        // Closed at the next open (112): the bar's own range is not counted, the close price is.
        broker.place(order(OrderKind::CloseAll, "close", true, None));
        broker.process(bar(112.0, 130.0, 60.0, 125.0), 2, 0.01);
        let trade = &broker.closed_trades[0];
        assert_eq!((trade.max_runup, trade.max_drawdown), (40.0, 30.0));
        assert!((trade.profit - 24.0).abs() < 1e-9);
    }

    #[test]
    fn a_shorts_run_up_is_how_far_it_fell() {
        let mut broker = Broker::new(settings());
        broker.place(order(OrderKind::Entry, "S", false, Some(1.0)));
        broker.process(bar(100.0, 104.0, 92.0, 95.0), 0, 0.01);
        assert_eq!(broker.open_trades[0].excursions(), (8.0, 4.0));
    }

    #[test]
    fn a_partial_close_gives_each_part_its_own_size_of_the_excursions() {
        let mut broker = Broker::new(settings());
        broker.place(order(OrderKind::Entry, "L", true, Some(4.0)));
        broker.process(bar(100.0, 110.0, 90.0, 105.0), 0, 0.01);
        let mut close = order(OrderKind::Close, "L", true, Some(1.0));
        close.quantity = Some(1.0);
        broker.place(close);
        broker.process(bar(105.0, 105.0, 105.0, 105.0), 1, 0.01);
        let closed = &broker.closed_trades[0];
        assert_eq!(
            (closed.quantity, closed.max_runup, closed.max_drawdown),
            (1.0, 10.0, 10.0)
        );
        assert_eq!(broker.open_trades[0].excursions(), (30.0, 30.0));
    }

    #[test]
    fn comments_travel_from_the_orders_to_the_trade() {
        let mut broker = Broker::new(settings());
        let mut entry = order(OrderKind::Entry, "L", true, Some(1.0));
        entry.comment = "breakout".to_owned();
        broker.place(entry);
        broker.process(bar(100.0, 101.0, 99.0, 100.0), 0, 0.01);
        assert_eq!(broker.open_trades[0].entry_comment, "breakout");
        let mut exit = order(OrderKind::Exit, "X", true, None);
        exit.stop = Some(95.0);
        exit.limit = Some(110.0);
        exit.comment = "plain".to_owned();
        exit.comment_loss = Some("stopped out".to_owned());
        exit.comment_profit = Some("target hit".to_owned());
        broker.place(exit.clone());
        // Falls through the stop first.
        broker.process(bar(100.0, 100.0, 94.0, 96.0), 1, 0.01);
        let trade = &broker.closed_trades[0];
        assert_eq!(trade.entry_comment, "breakout");
        assert_eq!(trade.exit_comment, "stopped out");
        assert_eq!(trade.comments(), "Entry: breakout\nExit: stopped out");
        // The same exit meets the limit on the next trade; without a specific comment it uses `comment`.
        let mut again = order(OrderKind::Entry, "L", true, Some(1.0));
        again.comment = String::new();
        broker.place(again);
        broker.process(bar(100.0, 101.0, 99.0, 100.0), 2, 0.01);
        exit.comment_profit = None;
        broker.place(exit);
        broker.process(bar(100.0, 112.0, 99.0, 111.0), 3, 0.01);
        assert_eq!(broker.closed_trades[1].exit_comment, "plain");
        assert_eq!(broker.closed_trades[1].comments(), "Exit: plain");
        // A close order and a close-all carry their own comments.
        let mut third = order(OrderKind::Entry, "L", true, Some(1.0));
        third.comment = "again".to_owned();
        broker.place(third);
        broker.process(bar(100.0, 101.0, 99.0, 100.0), 4, 0.01);
        let mut close = order(OrderKind::Close, "L", true, None);
        close.comment = "flat".to_owned();
        broker.place(close);
        broker.process(bar(100.0, 101.0, 99.0, 100.0), 5, 0.01);
        assert_eq!(broker.closed_trades[2].exit_comment, "flat");
        assert_eq!(broker.closed_trades[2].entry_comment, "again");
    }
}
