//! What the paper-trading tools return: accounts with their metrics, positions, working orders,
//! fills and closed trades, as typed values built from the paper-trading model. Read-only: no
//! tool here places, changes or cancels an order. Free of egui and HTTP; the app-thread glue that
//! reads the `PaperHost` is in `host::mcp`.

pub mod tools;

use crate::mcp::chart::data::iso;
use crate::model::paper::{
    PaperAccount, PaperAccountMetrics, PaperClosedTrade, PaperCurrency, PaperFill, PaperOrder,
    PaperOrderRole, PaperOrderSide, PaperOrderStatus, PaperOrderType, PaperPosition,
    PaperPositionSide, PaperTimeInForce,
};
use schemars::JsonSchema;
use serde::Serialize;

/// Rows one history page holds at most.
pub const MAX_HISTORY_PAGE: usize = 200;
pub const DEFAULT_HISTORY_PAGE: usize = 50;

fn currency(currency: PaperCurrency) -> &'static str {
    match currency {
        PaperCurrency::Usd => "usd",
        PaperCurrency::Eur => "eur",
        PaperCurrency::Gbp => "gbp",
        PaperCurrency::Jpy => "jpy",
        PaperCurrency::Usdt => "usdt",
        PaperCurrency::Usdc => "usdc",
    }
}

fn side(side: PaperOrderSide) -> &'static str {
    match side {
        PaperOrderSide::Buy => "buy",
        PaperOrderSide::Sell => "sell",
    }
}

fn position_side(side: PaperPositionSide) -> &'static str {
    match side {
        PaperPositionSide::Long => "long",
        PaperPositionSide::Short => "short",
    }
}

fn kind(kind: PaperOrderType) -> &'static str {
    match kind {
        PaperOrderType::Market => "market",
        PaperOrderType::Limit => "limit",
        PaperOrderType::Stop => "stop",
        PaperOrderType::StopLimit => "stop_limit",
    }
}

fn status(status: PaperOrderStatus) -> &'static str {
    match status {
        PaperOrderStatus::PendingSubmission => "pending_submission",
        PaperOrderStatus::Working => "working",
        PaperOrderStatus::PartiallyFilled => "partially_filled",
        PaperOrderStatus::Filled => "filled",
        PaperOrderStatus::PendingCancel => "pending_cancel",
        PaperOrderStatus::Canceled => "canceled",
        PaperOrderStatus::Rejected => "rejected",
        PaperOrderStatus::Expired => "expired",
    }
}

fn role(role: PaperOrderRole) -> &'static str {
    match role {
        PaperOrderRole::Entry => "entry",
        PaperOrderRole::TakeProfit => "take_profit",
        PaperOrderRole::StopLoss => "stop_loss",
    }
}

fn time_in_force(tif: PaperTimeInForce) -> &'static str {
    match tif {
        PaperTimeInForce::Day => "day",
        PaperTimeInForce::GoodTilCanceled => "good_til_canceled",
    }
}

fn finite(value: f64) -> Option<f64> {
    value.is_finite().then_some(value)
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct PaperAccountInfo {
    /// Pass it to the other paper tools.
    pub account_id: String,
    pub name: String,
    /// `usd`, `eur`, `gbp`, `jpy`, `usdt` or `usdc`. Every amount below is in it.
    pub currency: String,
    pub initial_balance: f64,
    /// Cash, before open positions.
    pub cash_balance: f64,
    pub realized_pnl: f64,
    pub unrealized_pnl: f64,
    /// Cash plus unrealized profit and loss.
    pub equity: f64,
    pub position_margin: f64,
    pub orders_margin: f64,
    pub available_funds: f64,
    pub margin_buffer: f64,
    /// The account the app has selected.
    pub selected: bool,
    /// Set when another Mac runs this account (iCloud Sync): this one does not fill its orders
    /// until the user takes it over. The figures are as last synced.
    pub run_by_another_mac: Option<String>,
}

pub fn account_info(
    account: &PaperAccount,
    metrics: &PaperAccountMetrics,
    selected: bool,
    run_by_another_mac: Option<String>,
) -> PaperAccountInfo {
    PaperAccountInfo {
        account_id: account.id.to_string(),
        name: account.name.clone(),
        currency: currency(account.base_currency).to_owned(),
        initial_balance: account.initial_balance,
        cash_balance: metrics.balance,
        realized_pnl: metrics.realized_pnl,
        unrealized_pnl: metrics.unrealized_pnl,
        equity: metrics.equity,
        position_margin: metrics.position_margin,
        orders_margin: metrics.orders_margin,
        available_funds: metrics.available_funds,
        margin_buffer: metrics.margin_buffer,
        selected,
        run_by_another_mac,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct PaperAccountsResult {
    pub accounts: Vec<PaperAccountInfo>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct PaperPositionInfo {
    /// Source-qualified market, e.g. `BINANCE:BTCUSDT`.
    pub instrument: String,
    pub display_name: String,
    /// `long` or `short`.
    pub side: String,
    /// Always positive; `side` says which way.
    pub quantity: f64,
    pub average_entry_price: f64,
    /// The price the position is marked at: the bid for a long, the ask for a short, else the last
    /// trade. Null while there is no quote, when the profit and loss below cannot be trusted.
    pub mark_price: Option<f64>,
    pub unrealized_pnl: Option<f64>,
    /// Unrealized profit and loss over the cost of the position, in percent.
    pub return_percent: Option<f64>,
    /// Profit and loss already taken on this position by partial closes, before commissions.
    pub realized_gross_pnl: f64,
    pub commissions: f64,
    pub opened_at: String,
}

pub fn position_info(
    position: &PaperPosition,
    mark: Option<f64>,
    unrealized_pnl: f64,
    return_ratio: Option<f64>,
) -> PaperPositionInfo {
    PaperPositionInfo {
        instrument: position.instrument.key.clone(),
        display_name: position.instrument.display_name.clone(),
        side: position_side(position.side()).to_owned(),
        quantity: position.quantity(),
        average_entry_price: position.average_entry_price,
        mark_price: mark.and_then(finite),
        unrealized_pnl: mark.and(finite(unrealized_pnl)),
        return_percent: return_ratio.and_then(finite).map(|ratio| ratio * 100.0),
        realized_gross_pnl: position.realized_gross_pnl,
        commissions: position.commissions,
        opened_at: iso(position.opened_at),
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct PaperOrderInfo {
    pub order_id: String,
    pub instrument: String,
    /// `buy` or `sell`.
    pub side: String,
    /// `market`, `limit`, `stop` or `stop_limit`.
    pub kind: String,
    pub quantity: f64,
    pub filled_quantity: f64,
    pub remaining_quantity: f64,
    pub limit_price: Option<f64>,
    pub stop_price: Option<f64>,
    pub average_fill_price: Option<f64>,
    /// `day` or `good_til_canceled`.
    pub time_in_force: String,
    /// `pending_submission`, `working`, `partially_filled`, `filled`, `pending_cancel`,
    /// `canceled`, `rejected` or `expired`.
    pub status: String,
    /// `entry`, or `take_profit` / `stop_loss` for the exit orders of a bracket.
    pub role: String,
    /// The entry order a bracket exit belongs to.
    pub parent_order_id: Option<String>,
    pub reserved_margin: f64,
    pub rejection_reason: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

pub fn order_info(order: &PaperOrder) -> PaperOrderInfo {
    PaperOrderInfo {
        order_id: order.id.to_string(),
        instrument: order.instrument.key.clone(),
        side: side(order.side).to_owned(),
        kind: kind(order.kind).to_owned(),
        quantity: order.original_quantity,
        filled_quantity: order.filled_quantity,
        remaining_quantity: order.remaining_quantity(),
        limit_price: order.limit_price,
        stop_price: order.stop_price,
        average_fill_price: order.average_fill_price,
        time_in_force: time_in_force(order.time_in_force).to_owned(),
        status: status(order.status).to_owned(),
        role: role(order.role).to_owned(),
        parent_order_id: order.parent_order_id.map(|id| id.to_string()),
        reserved_margin: order.reserved_margin,
        rejection_reason: order.rejection_reason.clone(),
        created_at: iso(order.created_at),
        updated_at: iso(order.updated_at),
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct PaperAccountDetail {
    pub account: PaperAccountInfo,
    pub positions: Vec<PaperPositionInfo>,
    /// Orders that can still fill: pending, working or partly filled.
    pub open_orders: Vec<PaperOrderInfo>,
}

pub fn is_open(order: &PaperOrder) -> bool {
    order.status.is_working() || order.status == PaperOrderStatus::PendingSubmission
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct PaperFillInfo {
    pub fill_id: String,
    pub order_id: String,
    pub instrument: String,
    pub side: String,
    pub quantity: f64,
    pub price: f64,
    pub commission: f64,
    /// `bid_ask`, `last_price_fallback` or `bar_fallback`: what the price was taken from.
    pub price_source: String,
    pub time: String,
}

pub fn fill_info(fill: &PaperFill) -> PaperFillInfo {
    use crate::model::paper::PaperFillPriceSource as Source;
    PaperFillInfo {
        fill_id: fill.id.to_string(),
        order_id: fill.order_id.to_string(),
        instrument: fill.instrument.key.clone(),
        side: side(fill.side).to_owned(),
        quantity: fill.quantity,
        price: fill.price,
        commission: fill.commission,
        price_source: match fill.price_source {
            Source::BidAsk => "bid_ask",
            Source::LastPriceFallback => "last_price_fallback",
            Source::BarFallback => "bar_fallback",
        }
        .to_owned(),
        time: iso(fill.timestamp),
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct PaperFillsResult {
    pub account_id: String,
    /// Newest first.
    pub fills: Vec<PaperFillInfo>,
    pub offset: usize,
    /// All fills of the account.
    pub total: usize,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct PaperClosedTradeInfo {
    pub trade_id: String,
    pub instrument: String,
    pub side: String,
    pub quantity: f64,
    pub entry_price: f64,
    pub exit_price: f64,
    pub entered_at: String,
    pub exited_at: String,
    pub gross_pnl: f64,
    pub commission: f64,
    /// Gross profit and loss less commission.
    pub net_pnl: f64,
}

pub fn closed_trade_info(trade: &PaperClosedTrade) -> PaperClosedTradeInfo {
    PaperClosedTradeInfo {
        trade_id: trade.id.to_string(),
        instrument: trade.instrument.key.clone(),
        side: position_side(trade.side).to_owned(),
        quantity: trade.quantity,
        entry_price: trade.entry_price,
        exit_price: trade.exit_price,
        entered_at: iso(trade.entry_timestamp),
        exited_at: iso(trade.exit_timestamp),
        gross_pnl: trade.gross_pnl,
        commission: trade.commission,
        net_pnl: trade.net_pnl(),
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct PaperTradesResult {
    pub account_id: String,
    /// Newest exit first.
    pub trades: Vec<PaperClosedTradeInfo>,
    pub offset: usize,
    /// All closed trades of the account.
    pub total: usize,
    pub truncated: bool,
    /// Across all of them, not only this page.
    pub winners: usize,
    pub losers: usize,
    pub total_net_pnl: f64,
}

/// One page of `rows`, newest first: `rows` come oldest first as the engine keeps them.
pub fn newest_first_page<T, R>(
    rows: &[T],
    offset: usize,
    limit: usize,
    map: impl Fn(&T) -> R,
) -> (Vec<R>, bool) {
    let page: Vec<R> = rows
        .iter()
        .rev()
        .skip(offset)
        .take(limit)
        .map(map)
        .collect();
    let truncated = offset + page.len() < rows.len();
    (page, truncated)
}

pub fn trades_result(
    account_id: u64,
    trades: &[&PaperClosedTrade],
    offset: usize,
    limit: usize,
) -> PaperTradesResult {
    let mut ordered: Vec<&&PaperClosedTrade> = trades.iter().collect();
    ordered.sort_by_key(|trade| trade.exit_timestamp);
    let (page, truncated) =
        newest_first_page(&ordered, offset, limit, |trade| closed_trade_info(trade));
    PaperTradesResult {
        account_id: account_id.to_string(),
        trades: page,
        offset,
        total: trades.len(),
        truncated,
        winners: trades.iter().filter(|trade| trade.net_pnl() > 0.0).count(),
        losers: trades.iter().filter(|trade| trade.net_pnl() < 0.0).count(),
        total_net_pnl: trades.iter().map(|trade| trade.net_pnl()).sum(),
    }
}

pub fn fills_result(
    account_id: u64,
    fills: &[&PaperFill],
    offset: usize,
    limit: usize,
) -> PaperFillsResult {
    let mut ordered: Vec<&&PaperFill> = fills.iter().collect();
    ordered.sort_by_key(|fill| fill.timestamp);
    let (page, truncated) = newest_first_page(&ordered, offset, limit, |fill| fill_info(fill));
    PaperFillsResult {
        account_id: account_id.to_string(),
        fills: page,
        offset,
        total: fills.len(),
        truncated,
    }
}

/// What placing an order did.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct PaperOrderResult {
    /// `placed`, `already_placed` (the same `request_id` was placed before: that order is
    /// returned and nothing new was submitted) or `would_place` (a dry run: the order is valid
    /// and nothing was submitted).
    pub status: String,
    /// The order as it stands after submission: `working`, `filled`, `partially_filled`. Null for
    /// a dry run.
    pub order: Option<PaperOrderInfo>,
    /// The take-profit and stop-loss exit orders of a bracket, once the entry has filled.
    pub bracket_orders: Vec<PaperOrderInfo>,
    /// Fills the order produced at once (a market order, or a limit already marketable).
    pub fills: Vec<PaperFillInfo>,
    /// The account afterwards (cash, equity, margin, available funds). Null unless the
    /// credential also holds `paper.read`: placing orders does not grant sight of balances.
    pub account: Option<PaperAccountInfo>,
}

/// What changing, cancelling or closing did, one entry per order or position touched.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct PaperChangeResult {
    /// `modified`, `canceled`, `closed`, or `nothing_to_do` (no working order / open position).
    pub status: String,
    /// The orders acted on, as they are now.
    pub orders: Vec<PaperOrderInfo>,
    /// Fills produced (closing a position is a market order).
    pub fills: Vec<PaperFillInfo>,
    /// Anything that could not be done, with why. The rest still happened: these are separate
    /// orders and positions.
    pub failures: Vec<String>,
    /// The account afterwards. Null unless the credential also holds `paper.read`.
    pub account: Option<PaperAccountInfo>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DataSource;
    use crate::model::paper::PaperInstrument;

    fn instrument() -> PaperInstrument {
        PaperInstrument {
            key: "BINANCE:BTCUSDT".into(),
            symbol: "BTCUSDT".into(),
            display_name: "BTC/USDT".into(),
            source: DataSource::Binance,
            asset_class: Default::default(),
            quote_currency: PaperCurrency::Usdt,
            tick_size: 0.01,
            minimum_quantity: 0.0,
            quantity_increment: 0.0,
            contract_multiplier: 1.0,
            point_value: 1.0,
            expiration: None,
        }
    }

    fn trade(id: u64, exit: i64, gross: f64, commission: f64) -> PaperClosedTrade {
        PaperClosedTrade {
            id,
            account_id: 1,
            instrument: instrument(),
            side: PaperPositionSide::Long,
            entry_timestamp: exit - 1_000,
            exit_timestamp: exit,
            entry_price: 100.0,
            exit_price: 110.0,
            quantity: 1.0,
            gross_pnl: gross,
            commission,
        }
    }

    #[test]
    fn a_position_without_a_quote_reports_no_profit_rather_than_a_made_up_one() {
        let position = PaperPosition {
            account_id: 1,
            instrument: instrument(),
            signed_quantity: -2.0,
            average_entry_price: 100.0,
            realized_gross_pnl: 0.0,
            commissions: 1.0,
            opened_at: 1_760_918_400_000,
            updated_at: 0,
        };
        let unmarked = position_info(&position, None, -1000.0, None);
        assert_eq!(unmarked.side, "short");
        assert_eq!(unmarked.quantity, 2.0);
        assert_eq!(unmarked.mark_price, None);
        assert_eq!(unmarked.unrealized_pnl, None);
        assert_eq!(unmarked.return_percent, None);
        let marked = position_info(&position, Some(95.0), 10.0, Some(0.05));
        assert_eq!(marked.unrealized_pnl, Some(10.0));
        assert_eq!(marked.return_percent, Some(5.0));
        assert_eq!(marked.opened_at, "2025-10-20T00:00:00Z");
    }

    #[test]
    fn closed_trades_are_newest_exit_first_with_totals_over_all_of_them() {
        let trades = [
            trade(1, 3_000, 10.0, 1.0),
            trade(2, 1_000, -5.0, 1.0),
            trade(3, 2_000, 4.0, 0.0),
        ];
        let refs: Vec<&PaperClosedTrade> = trades.iter().collect();
        let result = trades_result(1, &refs, 0, 2);
        let ids: Vec<_> = result.trades.iter().map(|t| t.trade_id.as_str()).collect();
        assert_eq!(ids, ["1", "3"]);
        assert!(result.truncated);
        assert_eq!((result.winners, result.losers, result.total), (2, 1, 3));
        assert!((result.total_net_pnl - 7.0).abs() < 1e-9);
        let rest = trades_result(1, &refs, 2, 2);
        assert_eq!(rest.trades.len(), 1);
        assert!(!rest.truncated);
        assert_eq!(rest.trades[0].net_pnl, -6.0);
    }

    #[test]
    fn an_order_reports_what_is_left_and_whether_it_can_still_fill() {
        let mut order = PaperOrder {
            id: 7,
            account_id: 1,
            instrument: instrument(),
            side: PaperOrderSide::Buy,
            kind: PaperOrderType::Limit,
            original_quantity: 3.0,
            filled_quantity: 1.0,
            average_fill_price: Some(99.0),
            limit_price: Some(99.0),
            stop_price: None,
            time_in_force: PaperTimeInForce::Day,
            status: PaperOrderStatus::PartiallyFilled,
            role: PaperOrderRole::Entry,
            parent_order_id: None,
            oco_group_id: None,
            stop_triggered: false,
            reserved_margin: 0.0,
            rejection_reason: None,
            created_at: 0,
            updated_at: 0,
        };
        let info = order_info(&order);
        assert_eq!(info.remaining_quantity, 2.0);
        assert_eq!(
            (info.kind.as_str(), info.status.as_str()),
            ("limit", "partially_filled")
        );
        assert!(is_open(&order));
        order.status = PaperOrderStatus::Filled;
        assert!(!is_open(&order));
        order.status = PaperOrderStatus::Rejected;
        assert!(!is_open(&order));
    }
}
