//! `strategy.entry`/`.order`/`.exit`/`.close`/`.close_all`/`.cancel`/`.cancel_all`, ported from
//! `PineRuntimeSession+Strategy.swift`. Each call places (or cancels) an order on the per-bar broker
//! held in `RuntimeState`; the broker emulator fills them as the bars advance. `strategy.risk.*`
//! limits are accepted and ignored, matching Pine. The `strategy.*` *identifiers*
//! (`strategy.position_size`, …) are read through [`strategy_value`], which the session's
//! identifier resolver calls — they are not function calls.

use super::broker::{Broker, Order, OrderKind};
use super::drawings::{Arg, bind};
use super::runtime_value::RuntimeValue;
use super::state::RuntimeState;
use super::token::{Diagnostic, DiagnosticCategory, SourceRange};

/// Whether `name` is a `strategy.*` function this module handles.
pub fn handles(name: &str) -> bool {
    name.starts_with("strategy.risk.")
        || matches!(
            name,
            "strategy.entry"
                | "strategy.order"
                | "strategy.exit"
                | "strategy.close"
                | "strategy.close_all"
                | "strategy.cancel"
                | "strategy.cancel_all"
        )
}

/// Runs one `strategy.*` call. `is_strategy` is the session's `is_strategy()` flag.
pub fn call(
    name: &str,
    args: &[Arg],
    state: &mut RuntimeState,
    is_strategy: bool,
    range: SourceRange,
) -> Result<RuntimeValue, Diagnostic> {
    if !is_strategy {
        return Err(runtime_error(
            "PINE4015",
            format!("{name} is only available in strategy() scripts."),
            range,
        ));
    }
    match name {
        "strategy.entry" | "strategy.order" => place_entry(name, args, state),
        "strategy.exit" => place_exit(args, state, range),
        "strategy.close" => place_close(args, state),
        "strategy.close_all" => {
            state.broker.place(order(
                OrderKind::CloseAll,
                "Close position order".to_owned(),
            ));
            Ok(RuntimeValue::Void)
        }
        "strategy.cancel" => {
            if let Some(id) = bind(args, &["id"])
                .get("id")
                .and_then(RuntimeValue::text_value)
            {
                state.broker.cancel(id);
            }
            Ok(RuntimeValue::Void)
        }
        "strategy.cancel_all" => {
            state.broker.cancel_all();
            Ok(RuntimeValue::Void)
        }
        _ if name.starts_with("strategy.risk.") => Ok(RuntimeValue::Void),
        _ => Err(runtime_error(
            "PINE4007",
            format!("Unknown or unsupported function '{name}'."),
            range,
        )),
    }
}

/// `strategy.closedtrades.exit_price(n)`, `strategy.opentrades.profit(n)`, …: one property of the
/// `trade_number`th trade (0 is the oldest). `na` for a trade that does not exist. None when `name`
/// is not a property this engine tracks (per-trade run-up and drawdown are not).
pub fn trade_property(
    name: &str,
    trade_number: Option<i64>,
    broker: &Broker,
    close: f64,
) -> Option<RuntimeValue> {
    let closed = name.strip_prefix("strategy.closedtrades.");
    let open = name.strip_prefix("strategy.opentrades.");
    let index = trade_number.and_then(|n| usize::try_from(n).ok());
    let float = |value: f64| RuntimeValue::Float(value);
    let signed = |is_long: bool, quantity: f64| if is_long { quantity } else { -quantity };
    let percent = |profit: f64, price: f64, quantity: f64| {
        profit_percent(profit, price, quantity).map_or(RuntimeValue::Na, float)
    };
    if let Some(property) = closed {
        let known = matches!(
            property,
            "entry_bar_index"
                | "entry_id"
                | "entry_price"
                | "entry_time"
                | "exit_bar_index"
                | "exit_id"
                | "exit_price"
                | "exit_time"
                | "size"
                | "profit"
                | "profit_percent"
                | "commission"
        );
        if !known {
            return None;
        }
        let Some(trade) = index.and_then(|i| broker.closed_trades.get(i)) else {
            return Some(RuntimeValue::Na);
        };
        return Some(match property {
            "entry_bar_index" => RuntimeValue::Int(trade.entry_bar as i64),
            "entry_id" => RuntimeValue::String(trade.entry_id.clone()),
            "entry_price" => float(trade.entry_price),
            "entry_time" => RuntimeValue::Int(trade.entry_time),
            "exit_bar_index" => RuntimeValue::Int(trade.exit_bar as i64),
            "exit_id" => RuntimeValue::String(trade.exit_id.clone()),
            "exit_price" => float(trade.exit_price),
            "exit_time" => RuntimeValue::Int(trade.exit_time),
            "size" => float(signed(trade.is_long, trade.quantity)),
            "profit" => float(trade.profit),
            "profit_percent" => percent(trade.profit, trade.entry_price, trade.quantity),
            _ => float(trade.commission),
        });
    }
    let property = open?;
    if !matches!(
        property,
        "entry_bar_index"
            | "entry_id"
            | "entry_price"
            | "entry_time"
            | "size"
            | "profit"
            | "profit_percent"
            | "commission"
    ) {
        return None;
    }
    let Some(trade) = index.and_then(|i| broker.open_trades.get(i)) else {
        return Some(RuntimeValue::Na);
    };
    let profit =
        if trade.is_long { 1.0 } else { -1.0 } * (close - trade.entry_price) * trade.quantity;
    Some(match property {
        "entry_bar_index" => RuntimeValue::Int(trade.entry_bar as i64),
        "entry_id" => RuntimeValue::String(trade.entry_id.clone()),
        "entry_price" => float(trade.entry_price),
        "entry_time" => RuntimeValue::Int(trade.entry_time),
        "size" => float(signed(trade.is_long, trade.quantity)),
        "profit" => float(profit),
        "profit_percent" => percent(profit, trade.entry_price, trade.quantity),
        _ => float(trade.entry_commission),
    })
}

/// A trade's profit as a percentage of what it put in (entry price × quantity); None when that is 0.
fn profit_percent(profit: f64, price: f64, quantity: f64) -> Option<f64> {
    let basis = price * quantity;
    (basis != 0.0).then(|| profit / basis * 100.0)
}

/// The mean of `values`, `na` when there are none.
fn mean(values: impl Iterator<Item = f64>) -> RuntimeValue {
    let (sum, count) = values.fold((0.0, 0_usize), |(sum, count), value| {
        (sum + value, count + 1)
    });
    if count == 0 {
        RuntimeValue::Na
    } else {
        RuntimeValue::Float(sum / count as f64)
    }
}

/// `value` as a percentage of `whole`, `na` when `whole` is not positive.
fn share(value: f64, whole: f64) -> RuntimeValue {
    if whole > 0.0 {
        RuntimeValue::Float(value / whole * 100.0)
    } else {
        RuntimeValue::Na
    }
}

/// The statistics over the closed trades and the equity curve. `na` where there is nothing to
/// average. Losses are negative numbers, so the average losing trade is negative while
/// `strategy.grossloss` is a positive total (inferred; the reference does not give the sign).
fn statistic(name: &str, broker: &Broker, close: f64) -> Option<RuntimeValue> {
    let trades = &broker.closed_trades;
    let wins = || trades.iter().filter(|trade| trade.profit > 0.0);
    let losses = || trades.iter().filter(|trade| trade.profit < 0.0);
    let percent = |trade: &super::strategy::Trade| {
        profit_percent(trade.profit, trade.entry_price, trade.quantity)
    };
    let initial = broker.settings.initial_capital;
    Some(match name {
        "strategy.avg_trade" => mean(trades.iter().map(|trade| trade.profit)),
        "strategy.avg_trade_percent" => mean(trades.iter().filter_map(percent)),
        "strategy.avg_winning_trade" => mean(wins().map(|trade| trade.profit)),
        "strategy.avg_winning_trade_percent" => mean(wins().filter_map(percent)),
        "strategy.avg_losing_trade" => mean(losses().map(|trade| trade.profit)),
        "strategy.avg_losing_trade_percent" => mean(losses().filter_map(percent)),
        "strategy.eventrades" => {
            RuntimeValue::Int(trades.iter().filter(|trade| trade.profit == 0.0).count() as i64)
        }
        "strategy.grossprofit_percent" => share(wins().map(|trade| trade.profit).sum(), initial),
        "strategy.netprofit_percent" => share(broker.net_profit, initial),
        // Unrealized profit against the equity the closed trades have realized.
        "strategy.openprofit_percent" => {
            share(broker.open_profit(close), initial + broker.net_profit)
        }
        "strategy.max_drawdown" => RuntimeValue::Float(broker.equity_extremes(close).max_drawdown),
        "strategy.max_drawdown_percent" => {
            RuntimeValue::Float(broker.equity_extremes(close).max_drawdown_percent)
        }
        "strategy.max_runup" => RuntimeValue::Float(broker.equity_extremes(close).max_runup),
        "strategy.max_runup_percent" => {
            RuntimeValue::Float(broker.equity_extremes(close).max_runup_percent)
        }
        // The largest single trade, closed or still open.
        "strategy.max_contracts_held_all"
        | "strategy.max_contracts_held_long"
        | "strategy.max_contracts_held_short" => {
            let wanted = |is_long: bool| match name {
                "strategy.max_contracts_held_long" => is_long,
                "strategy.max_contracts_held_short" => !is_long,
                _ => true,
            };
            RuntimeValue::Float(
                trades
                    .iter()
                    .map(|trade| (trade.is_long, trade.quantity))
                    .chain(
                        broker
                            .open_trades
                            .iter()
                            .map(|trade| (trade.is_long, trade.quantity)),
                    )
                    .filter(|(is_long, _)| wanted(*is_long))
                    .fold(0.0, |most, (_, quantity)| f64::max(most, quantity)),
            )
        }
        // The order that opened the position is the oldest of its open trades; "" when flat.
        "strategy.position_entry_name" => RuntimeValue::String(
            broker
                .open_trades
                .first()
                .map(|trade| trade.entry_id.clone())
                .unwrap_or_default(),
        ),
        // Every closed trade is kept, so the oldest is trade 0.
        "strategy.closedtrades.first_index" => RuntimeValue::Int(0),
        // What the open trades put in; there is no margin, so none of it is borrowed.
        "strategy.opentrades.capital_held" => RuntimeValue::Float(
            broker
                .open_trades
                .iter()
                .map(|trade| trade.entry_price * trade.quantity)
                .sum(),
        ),
        _ => return None,
    })
}

/// The value a `strategy.*` identifier reads, or None when `name` is not one of them.
pub fn strategy_value(name: &str, broker: &Broker, close: f64) -> Option<RuntimeValue> {
    if let Some(value) = statistic(name, broker, close) {
        return Some(value);
    }
    match name {
        "strategy.position_size" => Some(RuntimeValue::Float(broker.position_size())),
        "strategy.position_avg_price" => {
            if broker.position_size() == 0.0 {
                Some(RuntimeValue::Na)
            } else {
                Some(RuntimeValue::Float(broker.average_price()))
            }
        }
        "strategy.equity" => Some(RuntimeValue::Float(broker.equity(close))),
        "strategy.netprofit" => Some(RuntimeValue::Float(broker.net_profit)),
        "strategy.openprofit" => Some(RuntimeValue::Float(broker.open_profit(close))),
        "strategy.initial_capital" => Some(RuntimeValue::Float(broker.settings.initial_capital)),
        "strategy.closedtrades" => Some(RuntimeValue::Int(broker.closed_trades.len() as i64)),
        "strategy.opentrades" => Some(RuntimeValue::Int(broker.open_trades.len() as i64)),
        "strategy.wintrades" => Some(RuntimeValue::Int(
            broker
                .closed_trades
                .iter()
                .filter(|trade| trade.profit > 0.0)
                .count() as i64,
        )),
        "strategy.losstrades" => Some(RuntimeValue::Int(
            broker
                .closed_trades
                .iter()
                .filter(|trade| trade.profit < 0.0)
                .count() as i64,
        )),
        "strategy.grossprofit" => Some(RuntimeValue::Float(
            broker
                .closed_trades
                .iter()
                .filter(|trade| trade.profit > 0.0)
                .map(|trade| trade.profit)
                .sum(),
        )),
        "strategy.grossloss" => Some(RuntimeValue::Float(
            -broker
                .closed_trades
                .iter()
                .filter(|trade| trade.profit < 0.0)
                .map(|trade| trade.profit)
                .sum::<f64>(),
        )),
        _ => None,
    }
}

fn runtime_error(code: &str, message: String, range: SourceRange) -> Diagnostic {
    Diagnostic::error(code, DiagnosticCategory::Runtime, message, range)
}

/// A quantity argument: finite and positive, else None.
fn positive(value: Option<&RuntimeValue>) -> Option<f64> {
    let number = value.and_then(RuntimeValue::number)?;
    (number.is_finite() && number > 0.0).then_some(number)
}

fn order(kind: OrderKind, id: String) -> Order {
    Order {
        kind,
        id,
        is_long: false,
        quantity: None,
        quantity_percent: None,
        limit: None,
        stop: None,
        from_entry: None,
        profit_ticks: None,
        loss_ticks: None,
        sequence: 0,
    }
}

fn place_entry(
    name: &str,
    args: &[Arg],
    state: &mut RuntimeState,
) -> Result<RuntimeValue, Diagnostic> {
    let b = bind(
        args,
        &[
            "id",
            "direction",
            "qty",
            "limit",
            "stop",
            "oca_name",
            "oca_type",
            "comment",
            "alert_message",
        ],
    );
    let Some(id) = b
        .get("id")
        .and_then(RuntimeValue::text_value)
        .map(str::to_owned)
    else {
        return Ok(RuntimeValue::Void);
    };
    let Some(direction) = b.get("direction").and_then(RuntimeValue::text_value) else {
        return Ok(RuntimeValue::Void);
    };
    let mut placed = order(
        if name == "strategy.entry" {
            OrderKind::Entry
        } else {
            OrderKind::Order
        },
        id,
    );
    placed.is_long = direction == "strategy.long";
    placed.quantity = positive(b.get("qty"));
    placed.limit = b.get("limit").and_then(RuntimeValue::number);
    placed.stop = b.get("stop").and_then(RuntimeValue::number);
    state.broker.place(placed);
    Ok(RuntimeValue::Void)
}

fn place_exit(
    args: &[Arg],
    state: &mut RuntimeState,
    range: SourceRange,
) -> Result<RuntimeValue, Diagnostic> {
    let b = bind(
        args,
        &[
            "id",
            "from_entry",
            "qty",
            "qty_percent",
            "profit",
            "limit",
            "loss",
            "stop",
            "trail_price",
            "trail_points",
            "trail_offset",
        ],
    );
    if b.get("trail_price")
        .and_then(RuntimeValue::number)
        .is_some()
        || b.get("trail_points")
            .and_then(RuntimeValue::number)
            .is_some()
    {
        return Err(Diagnostic::error(
            "PINE9005",
            DiagnosticCategory::Unsupported,
            "strategy.exit trailing stops are not supported yet.",
            range,
        ));
    }
    let Some(id) = b
        .get("id")
        .and_then(RuntimeValue::text_value)
        .map(str::to_owned)
    else {
        return Ok(RuntimeValue::Void);
    };
    let mut placed = order(OrderKind::Exit, id);
    placed.quantity = positive(b.get("qty"));
    placed.quantity_percent = positive(b.get("qty_percent"));
    placed.limit = b.get("limit").and_then(RuntimeValue::number);
    placed.stop = b.get("stop").and_then(RuntimeValue::number);
    placed.from_entry = b
        .get("from_entry")
        .and_then(RuntimeValue::text_value)
        .map(str::to_owned);
    placed.profit_ticks = positive(b.get("profit"));
    placed.loss_ticks = positive(b.get("loss"));
    state.broker.place(placed);
    Ok(RuntimeValue::Void)
}

fn place_close(args: &[Arg], state: &mut RuntimeState) -> Result<RuntimeValue, Diagnostic> {
    let b = bind(args, &["id", "comment", "qty", "qty_percent"]);
    let Some(id) = b
        .get("id")
        .and_then(RuntimeValue::text_value)
        .map(str::to_owned)
    else {
        return Ok(RuntimeValue::Void);
    };
    let mut placed = order(OrderKind::Close, id);
    placed.quantity = positive(b.get("qty"));
    placed.quantity_percent = positive(b.get("qty_percent"));
    state.broker.place(placed);
    Ok(RuntimeValue::Void)
}

#[cfg(test)]
mod tests {
    use super::*;
    use RuntimeValue::{Float, Na, String as Str};

    fn arg(value: RuntimeValue) -> Arg {
        Arg { name: None, value }
    }

    fn run(
        state: &mut RuntimeState,
        name: &str,
        args: &[Arg],
        is_strategy: bool,
    ) -> Result<RuntimeValue, Diagnostic> {
        call(name, args, state, is_strategy, SourceRange::ZERO)
    }

    fn s(text: &str) -> RuntimeValue {
        Str(text.to_owned())
    }

    #[test]
    fn entry_places_a_long_order() {
        let mut state = RuntimeState::default();
        run(
            &mut state,
            "strategy.entry",
            &[arg(s("Long")), arg(s("strategy.long")), arg(Float(2.0))],
            true,
        )
        .unwrap();
        let placed = &state.broker.orders[0];
        assert_eq!(placed.kind, OrderKind::Entry);
        assert!(placed.is_long);
        assert_eq!(placed.quantity, Some(2.0));
    }

    #[test]
    fn outside_a_strategy_is_pine4015() {
        let mut state = RuntimeState::default();
        let error = run(&mut state, "strategy.entry", &[arg(s("L"))], false).unwrap_err();
        assert_eq!(error.code, "PINE4015");
    }

    #[test]
    fn trailing_stop_is_unsupported() {
        let mut state = RuntimeState::default();
        let error = run(
            &mut state,
            "strategy.exit",
            &[
                arg(s("x")),
                Arg {
                    name: Some("trail_price".to_owned()),
                    value: Float(1.0),
                },
            ],
            true,
        )
        .unwrap_err();
        assert_eq!(error.code, "PINE9005");
    }

    #[test]
    fn close_all_and_cancel() {
        let mut state = RuntimeState::default();
        run(&mut state, "strategy.close_all", &[], true).unwrap();
        assert_eq!(state.broker.orders[0].kind, OrderKind::CloseAll);
        run(
            &mut state,
            "strategy.entry",
            &[arg(s("E")), arg(s("strategy.long"))],
            true,
        )
        .unwrap();
        run(&mut state, "strategy.cancel", &[arg(s("E"))], true).unwrap();
        assert!(
            state
                .broker
                .orders
                .iter()
                .all(|o| o.kind != OrderKind::Entry)
        );
    }

    #[test]
    fn strategy_values_read_the_broker() {
        let broker = RuntimeState::default().broker;
        assert_eq!(
            strategy_value("strategy.initial_capital", &broker, 0.0),
            Some(Float(1_000_000.0))
        );
        assert_eq!(
            strategy_value("strategy.closedtrades", &broker, 0.0),
            Some(RuntimeValue::Int(0))
        );
        assert_eq!(
            strategy_value("strategy.position_avg_price", &broker, 0.0),
            Some(Na)
        );
        assert_eq!(strategy_value("strategy.nope", &broker, 0.0), None);
        assert!(handles("strategy.entry") && handles("strategy.risk.max_position_size"));
        assert!(!handles("strategy.position_size") && !handles("plot"));
    }
}
