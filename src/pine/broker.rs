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

/// How an order in an OCA group (the orders sharing an `oca_name`) reacts when another of them fills.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OcaKind {
    /// The other orders of the group are cancelled.
    Cancel,
    /// The other orders of the group shrink by the quantity that filled, and go when none is left.
    Reduce,
}

/// A limit a risk rule measures, in money or as a percentage of the equity it is measured from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RiskLimit {
    Cash(f64),
    PercentOfEquity(f64),
}

impl RiskLimit {
    /// Whether a fall of `loss` from an equity of `peak` has reached the limit.
    fn reached(self, loss: f64, peak: f64) -> bool {
        match self {
            Self::Cash(amount) => loss >= amount - EPSILON,
            Self::PercentOfEquity(percent) => {
                peak > 0.0 && loss / peak * 100.0 >= percent - EPSILON
            }
        }
    }
}

/// What the script's `strategy.risk.*` calls set. Each call states its rule afresh, so the values
/// are simply the latest.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RiskRules {
    /// `allow_entry_in`: Some(true) allows only long entries, Some(false) only short ones.
    pub allowed_direction: Option<bool>,
    /// `max_position_size`: the most contracts a position may hold.
    pub max_position_size: Option<f64>,
    pub max_drawdown: Option<RiskLimit>,
    pub max_intraday_loss: Option<RiskLimit>,
    pub max_intraday_filled_orders: Option<usize>,
    pub max_consecutive_loss_days: Option<usize>,
}

/// Whether a risk rule has stopped the strategy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Halt {
    Running,
    /// Until the day ends (`max_intraday_loss`, `max_intraday_filled_orders`).
    ForTheDay,
    /// For good (`max_drawdown`, `max_cons_loss_days`).
    Forever,
}

/// What the broker keeps of the trading day a risk rule measures.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Day {
    number: i64,
    start_equity: f64,
    peak_equity: f64,
    last_equity: f64,
    fills: usize,
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
    /// The OCA group the order belongs to, and how it reacts to a fill of another member.
    pub oca: Option<(String, OcaKind)>,
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
    pub risk: RiskRules,
    halt: Halt,
    day: Option<Day>,
    consecutive_loss_days: usize,
    /// The time zone the trading days are counted in (the exchange's).
    pub zone: Option<&'static str>,
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
            risk: RiskRules::default(),
            halt: Halt::Running,
            day: None,
            consecutive_loss_days: 0,
            zone: None,
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

    // MARK: Risk rules

    /// The number of the trading day `millis` falls in, in the broker's zone.
    fn local_day(&self, millis: i64) -> i64 {
        let offset = super::time::zone_offset_secs(self.zone, millis);
        (millis + offset * 1_000).div_euclid(86_400_000)
    }

    /// Starts a new trading day when the bar opens in one: the last day's result counts towards the
    /// run of losing days, and a halt that lasted the day is lifted.
    fn roll_day(&mut self, open_time: i64) {
        let number = self.local_day(open_time);
        if self.day.is_some_and(|day| day.number == number) {
            return;
        }
        let start_equity = match self.day {
            Some(previous) => {
                if previous.last_equity < previous.start_equity - EPSILON {
                    self.consecutive_loss_days += 1;
                } else {
                    self.consecutive_loss_days = 0;
                }
                previous.last_equity
            }
            None => self.settings.initial_capital,
        };
        self.day = Some(Day {
            number,
            start_equity,
            peak_equity: start_equity,
            last_equity: start_equity,
            fills: 0,
        });
        if self.halt == Halt::ForTheDay {
            self.halt = Halt::Running;
        }
    }

    /// Counts a filled order against the day's limit.
    fn note_fill(&mut self) {
        if let Some(day) = self.day.as_mut() {
            day.fills += 1;
        }
    }

    /// After the bar's equity is known: stops the strategy when a risk rule's limit has been reached.
    /// Stopping cancels every pending order and closes every open trade at the bar's close.
    pub fn check_risk(&mut self, bar: &Kline, bar_index: usize, mintick: f64) {
        let equity = self.equity(bar.close);
        let Some(day) = self.day.as_mut() else {
            return;
        };
        day.peak_equity = day.peak_equity.max(equity);
        day.last_equity = equity;
        let day = *day;
        if self.halt == Halt::Forever {
            return;
        }
        let drawdown = self.extremes.peak() - equity;
        let forever = self
            .risk
            .max_drawdown
            .is_some_and(|limit| limit.reached(drawdown, self.extremes.peak()))
            || self.risk.max_consecutive_loss_days.is_some_and(|days| {
                // Today counts when it is a loss so far.
                let today = usize::from(equity < day.start_equity - EPSILON);
                days > 0 && self.consecutive_loss_days + today >= days
            });
        let for_the_day = self.halt == Halt::Running
            && (self
                .risk
                .max_intraday_loss
                .is_some_and(|limit| limit.reached(day.peak_equity - equity, day.peak_equity))
                || self
                    .risk
                    .max_intraday_filled_orders
                    .is_some_and(|count| day.fills >= count));
        let halt = if forever {
            Halt::Forever
        } else if for_the_day {
            Halt::ForTheDay
        } else {
            return;
        };
        self.orders.clear();
        let context = BarContext {
            bar: bar.clone(),
            bar_index,
            mintick,
        };
        let _ = self.close(
            &|_| true,
            None,
            bar.close,
            "Close position order",
            "",
            &context,
        );
        self.halt = halt;
    }

    // MARK: Order book

    /// Placing an order with the id of a pending one of the same family replaces it.
    pub fn place(&mut self, mut order: Order) {
        // A risk rule has stopped the strategy: it can place nothing until the rule lets it.
        if self.halt != Halt::Running {
            return;
        }
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
        self.roll_day(bar.open_time);
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

    /// The quantity an entry gets when the script gives none, if it filled at `price`.
    pub fn default_quantity(&self, price: f64) -> f64 {
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
                let mut filled = false;
                // `allow_entry_in`: an entry against the allowed direction only closes the other side.
                let forbidden = self
                    .risk
                    .allowed_direction
                    .is_some_and(|long| long != order.is_long);
                if opposes_position {
                    filled |= self.close(
                        &|trade| trade.is_long != order.is_long,
                        None,
                        price,
                        &order.id,
                        &order.comment,
                        context,
                    );
                } else if !forbidden {
                    let same_side = self
                        .open_trades
                        .iter()
                        .filter(|trade| trade.is_long == order.is_long)
                        .count();
                    if same_side >= self.settings.pyramiding.max(1) {
                        return;
                    }
                }
                let mut quantity = 0.0;
                if !forbidden {
                    quantity = order
                        .quantity
                        .unwrap_or_else(|| self.default_quantity(price));
                    // `max_position_size`: the position may not grow past it.
                    if let Some(most) = self.risk.max_position_size {
                        let held: f64 = self
                            .open_trades
                            .iter()
                            .filter(|trade| trade.is_long == order.is_long)
                            .map(|trade| trade.quantity)
                            .sum();
                        quantity = quantity.min((most - held).max(0.0));
                    }
                    filled |= self.open(&order, quantity, price, context);
                }
                if filled {
                    self.note_fill();
                    self.apply_oca(&order, quantity, price);
                }
            }
            OrderKind::Order => {
                let mut remaining = order
                    .quantity
                    .unwrap_or_else(|| self.default_quantity(price));
                let total = remaining;
                let mut filled = false;
                if opposes_position {
                    let reduce = remaining.min(position.abs());
                    filled |= self.close(
                        &|trade| trade.is_long != order.is_long,
                        Some(reduce),
                        price,
                        &order.id,
                        &order.comment,
                        context,
                    );
                    remaining -= reduce;
                }
                filled |= self.open(&order, remaining, price, context);
                if filled {
                    self.note_fill();
                    self.apply_oca(&order, total, price);
                }
            }
            OrderKind::Exit | OrderKind::Close | OrderKind::CloseAll => {}
        }
    }

    fn open(&mut self, order: &Order, quantity: f64, price: f64, context: &BarContext) -> bool {
        if !quantity.is_finite() || quantity <= EPSILON {
            return false;
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
        true
    }

    /// When an order of an OCA group fills `filled` contracts at `price`: the group's other pending
    /// entries are cancelled, or (for the reducing kind) shrink by the quantity and go at zero.
    fn apply_oca(&mut self, order: &Order, filled: f64, price: f64) {
        let Some((name, kind)) = order.oca.as_ref() else {
            return;
        };
        let in_group = |other: &Order| {
            other.kind.is_entry_like() && other.oca.as_ref().is_some_and(|(n, _)| n == name)
        };
        match kind {
            OcaKind::Cancel => self.orders.retain(|other| !in_group(other)),
            OcaKind::Reduce => {
                let defaults: Vec<(usize, f64)> = self
                    .orders
                    .iter()
                    .filter(|other| in_group(other))
                    .map(|other| {
                        (
                            other.sequence,
                            other
                                .quantity
                                .unwrap_or_else(|| self.default_quantity(price)),
                        )
                    })
                    .collect();
                for (sequence, quantity) in defaults {
                    let left = quantity - filled;
                    if left <= EPSILON {
                        self.orders.retain(|other| other.sequence != sequence);
                    } else if let Some(other) = self
                        .orders
                        .iter_mut()
                        .find(|other| other.sequence == sequence)
                    {
                        other.quantity = Some(left);
                    }
                }
            }
        }
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
        let closed = self.close(
            &|trade| trade.entry_id == order.id,
            quantity,
            self.slipped(price, !is_long, context.mintick),
            &format!("Close entry(s) order {}", order.id),
            &order.comment,
            context,
        );
        if closed {
            self.note_fill();
        }
    }

    fn execute_close_all(&mut self, order: Order, price: f64, context: &BarContext) {
        let is_long = self.position_size() > 0.0;
        let closed = self.close(
            &|_| true,
            None,
            self.slipped(price, !is_long, context.mintick),
            &order.id,
            &order.comment,
            context,
        );
        if closed {
            self.note_fill();
        }
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
    ) -> bool {
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
        charged_order
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
        let closed = self.close(
            &|t| t.entry_id == trade.entry_id && t.entry_bar == trade.entry_bar,
            Some(requested.unwrap_or(trade.quantity).min(trade.quantity)),
            price,
            &order.id,
            &comment,
            context,
        );
        if closed {
            self.note_fill();
        }
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
            oca: None,
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

    const DAY_MS: i64 = 86_400_000;

    /// A bar on day `day` (from 0) that opens, and closes, at `price`.
    fn day_bar(day: i64, price: f64) -> Kline {
        Kline {
            open_time: day * DAY_MS,
            close_time: day * DAY_MS + 3_599_999,
            ..bar(price, price, price, price)
        }
    }

    /// One bar the way a strategy run does it: orders fill, the equity is recorded, the rules are checked.
    fn step(broker: &mut Broker, bar: Kline, index: usize) {
        broker.process(bar.clone(), index, 0.01);
        broker.record_equity(bar.close);
        broker.check_risk(&bar, index, 0.01);
    }

    fn oca(name: &str, kind: OcaKind) -> Option<(String, OcaKind)> {
        Some((name.to_owned(), kind))
    }

    #[test]
    fn allow_entry_in_turns_the_other_side_into_a_close() {
        let mut broker = Broker::new(settings());
        broker.risk.allowed_direction = Some(true);
        broker.place(order(OrderKind::Entry, "L", true, Some(1.0)));
        step(&mut broker, day_bar(0, 100.0), 0);
        assert_eq!(broker.position_size(), 1.0);
        // A short entry may not open a short: it only closes the long.
        broker.place(order(OrderKind::Entry, "S", false, Some(1.0)));
        step(&mut broker, day_bar(0, 110.0), 1);
        assert_eq!(broker.position_size(), 0.0);
        assert_eq!(broker.closed_trades.len(), 1);
        broker.place(order(OrderKind::Entry, "S", false, Some(1.0)));
        step(&mut broker, day_bar(0, 110.0), 2);
        assert_eq!(
            broker.position_size(),
            0.0,
            "and with nothing to close it does nothing"
        );
        // Short only is the mirror image; all lifts the rule.
        broker.risk.allowed_direction = Some(false);
        broker.place(order(OrderKind::Entry, "L", true, Some(1.0)));
        step(&mut broker, day_bar(0, 110.0), 3);
        assert_eq!(broker.position_size(), 0.0);
        broker.risk.allowed_direction = None;
        broker.place(order(OrderKind::Entry, "L", true, Some(1.0)));
        step(&mut broker, day_bar(0, 110.0), 4);
        assert_eq!(broker.position_size(), 1.0);
    }

    #[test]
    fn max_position_size_caps_what_an_entry_adds() {
        let mut broker = Broker::new(StrategySettings {
            pyramiding: 3,
            ..settings()
        });
        broker.risk.max_position_size = Some(5.0);
        broker.place(order(OrderKind::Entry, "A", true, Some(4.0)));
        step(&mut broker, day_bar(0, 100.0), 0);
        broker.place(order(OrderKind::Entry, "B", true, Some(4.0)));
        step(&mut broker, day_bar(0, 100.0), 1);
        assert_eq!(
            broker.position_size(),
            5.0,
            "the second entry is cut to what is left"
        );
        broker.place(order(OrderKind::Entry, "C", true, Some(1.0)));
        step(&mut broker, day_bar(0, 100.0), 2);
        assert_eq!(broker.position_size(), 5.0);
        assert_eq!(broker.open_trades.len(), 2, "no empty trade is opened");
    }

    #[test]
    fn an_oca_cancel_group_loses_its_other_orders_when_one_fills() {
        let mut broker = Broker::new(settings());
        let mut first = order(OrderKind::Entry, "A", true, Some(1.0));
        first.limit = Some(95.0);
        first.oca = oca("g", OcaKind::Cancel);
        let mut second = order(OrderKind::Entry, "B", false, Some(1.0));
        second.limit = Some(120.0);
        second.oca = oca("g", OcaKind::Cancel);
        let mut apart = order(OrderKind::Entry, "C", true, Some(1.0));
        apart.limit = Some(80.0);
        broker.place(first);
        broker.place(second);
        broker.place(apart);
        step(&mut broker, bar(100.0, 101.0, 94.0, 96.0), 0);
        assert_eq!(broker.open_trades.len(), 1);
        assert_eq!(broker.open_trades[0].entry_id, "A");
        let pending: Vec<&str> = broker.orders.iter().map(|o| o.id.as_str()).collect();
        assert_eq!(
            pending,
            ["C"],
            "B went with A's fill, C was never in the group"
        );
    }

    #[test]
    fn an_oca_reduce_group_shrinks_by_what_filled() {
        let mut broker = Broker::new(StrategySettings {
            pyramiding: 3,
            ..settings()
        });
        let mut first = order(OrderKind::Entry, "A", true, Some(5.0));
        first.limit = Some(95.0);
        first.oca = oca("g", OcaKind::Reduce);
        let mut second = order(OrderKind::Entry, "B", true, Some(8.0));
        second.limit = Some(90.0);
        second.oca = oca("g", OcaKind::Reduce);
        let mut third = order(OrderKind::Entry, "C", true, Some(4.0));
        third.limit = Some(85.0);
        third.oca = oca("g", OcaKind::Reduce);
        broker.place(first);
        broker.place(second);
        broker.place(third);
        step(&mut broker, bar(100.0, 101.0, 94.0, 96.0), 0);
        let left: Vec<(&str, Option<f64>)> = broker
            .orders
            .iter()
            .map(|o| (o.id.as_str(), o.quantity))
            .collect();
        // A filled 5: B has 3 left, C (4) is used up.
        assert_eq!(left, [("B", Some(3.0))]);
        step(&mut broker, bar(96.0, 97.0, 89.0, 90.0), 1);
        let held: f64 = broker.open_trades.iter().map(|t| t.quantity).sum();
        assert_eq!(held, 8.0, "5 and then the 3 B had left");
        assert!(broker.orders.is_empty());
    }

    #[test]
    fn max_drawdown_stops_the_strategy_for_good() {
        let mut broker = Broker::new(settings());
        broker.risk.max_drawdown = Some(RiskLimit::Cash(50.0));
        broker.place(order(OrderKind::Entry, "L", true, Some(1.0)));
        step(&mut broker, day_bar(0, 100.0), 0);
        // A pending order that the halt cancels.
        let mut waiting = order(OrderKind::Entry, "W", true, Some(1.0));
        waiting.limit = Some(1.0);
        broker.place(waiting);
        step(&mut broker, day_bar(0, 70.0), 1);
        assert!(
            broker.open_trades.len() == 1 && broker.orders.len() == 1,
            "30 down: not yet"
        );
        step(&mut broker, day_bar(0, 40.0), 2);
        assert!(broker.open_trades.is_empty(), "60 down closes the position");
        assert!(broker.orders.is_empty(), "and cancels what was waiting");
        assert_eq!(broker.closed_trades.len(), 1);
        broker.place(order(OrderKind::Entry, "again", true, Some(1.0)));
        assert!(broker.orders.is_empty(), "nothing can be placed any more");
        // Not even on a later day.
        step(&mut broker, day_bar(5, 40.0), 3);
        broker.place(order(OrderKind::Entry, "later", true, Some(1.0)));
        assert!(broker.orders.is_empty());
    }

    #[test]
    fn max_drawdown_as_a_percentage_of_the_peak_equity() {
        let mut broker = Broker::new(StrategySettings {
            initial_capital: 1000.0,
            ..settings()
        });
        broker.risk.max_drawdown = Some(RiskLimit::PercentOfEquity(10.0));
        broker.place(order(OrderKind::Entry, "L", true, Some(10.0)));
        step(&mut broker, day_bar(0, 100.0), 0);
        step(&mut broker, day_bar(0, 110.0), 1); // equity 1100: the peak
        step(&mut broker, day_bar(0, 101.0), 2); // 1010: 90 down, 8.2 %
        assert_eq!(broker.open_trades.len(), 1);
        step(&mut broker, day_bar(0, 99.0), 3); // 990: 110 down, 10 % of 1100
        assert!(broker.open_trades.is_empty());
    }

    #[test]
    fn max_intraday_loss_is_measured_from_the_days_best_equity_and_a_new_day_lifts_it() {
        let mut broker = Broker::new(settings());
        broker.risk.max_intraday_loss = Some(RiskLimit::Cash(20.0));
        broker.place(order(OrderKind::Entry, "L", true, Some(1.0)));
        step(&mut broker, day_bar(0, 100.0), 0);
        step(&mut broker, day_bar(0, 110.0), 1);
        step(&mut broker, day_bar(0, 95.0), 2);
        assert_eq!(broker.open_trades.len(), 1, "15 off the best, not yet");
        step(&mut broker, day_bar(0, 89.0), 3);
        assert!(broker.open_trades.is_empty(), "21 off the best: stopped");
        broker.place(order(OrderKind::Entry, "L2", true, Some(1.0)));
        assert!(broker.orders.is_empty(), "no new orders today");
        // The next day lifts the stop.
        step(&mut broker, day_bar(1, 89.0), 4);
        broker.place(order(OrderKind::Entry, "L3", true, Some(1.0)));
        assert_eq!(broker.orders.len(), 1);
    }

    #[test]
    fn max_intraday_filled_orders_counts_each_filled_order_once() {
        let mut broker = Broker::new(settings());
        broker.risk.max_intraday_filled_orders = Some(2);
        broker.place(order(OrderKind::Entry, "L", true, Some(1.0)));
        step(&mut broker, day_bar(0, 100.0), 0); // one filled order
        broker.place(order(OrderKind::CloseAll, "close", true, None));
        step(&mut broker, day_bar(0, 100.0), 1); // two: the limit
        broker.place(order(OrderKind::Entry, "L2", true, Some(1.0)));
        assert!(broker.orders.is_empty(), "stopped for the day");
        step(&mut broker, day_bar(1, 100.0), 2);
        broker.place(order(OrderKind::Entry, "L3", true, Some(1.0)));
        assert_eq!(broker.orders.len(), 1, "a new day, a new count");
    }

    #[test]
    fn max_cons_loss_days_stops_the_strategy_after_that_many_losing_days() {
        let mut broker = Broker::new(settings());
        broker.risk.max_consecutive_loss_days = Some(2);
        broker.place(order(OrderKind::Entry, "L", true, Some(1.0)));
        step(&mut broker, day_bar(0, 100.0), 0);
        step(&mut broker, day_bar(0, 98.0), 1);
        step(&mut broker, day_bar(1, 98.0), 2);
        step(&mut broker, day_bar(1, 96.0), 3);
        // Day 0 lost 2 and day 1 is losing 2 more: that is the second losing day.
        assert!(broker.open_trades.is_empty());
        broker.place(order(OrderKind::Entry, "again", true, Some(1.0)));
        assert!(broker.orders.is_empty());
    }

    #[test]
    fn a_winning_day_breaks_the_run_of_losing_days() {
        let mut broker = Broker::new(settings());
        broker.risk.max_consecutive_loss_days = Some(2);
        broker.place(order(OrderKind::Entry, "L", true, Some(1.0)));
        step(&mut broker, day_bar(0, 100.0), 0);
        step(&mut broker, day_bar(0, 98.0), 1); // day 0 loses
        step(&mut broker, day_bar(1, 98.0), 2);
        step(&mut broker, day_bar(1, 101.0), 3); // day 1 wins
        step(&mut broker, day_bar(2, 101.0), 4);
        step(&mut broker, day_bar(2, 100.0), 5); // day 2 loses: one day in a row
        assert_eq!(broker.open_trades.len(), 1);
    }

    #[test]
    fn the_default_entry_quantity_is_the_strategys_for_a_price() {
        let broker = Broker::new(StrategySettings {
            quantity_type: crate::pine::strategy::QuantityType::Cash,
            quantity_value: 1000.0,
            ..settings()
        });
        assert_eq!(broker.default_quantity(50.0), 20.0);
        assert_eq!(broker.default_quantity(0.0), 0.0);
    }
}
