//! The Paper Trading panel's "Export Trading Data": one account's orders, fills and closed
//! trades as three CSV files, in the columns the Swift app wrote. Pure text; the panel asks for a
//! folder and writes them.

use crate::model::paper::PaperTradingSnapshot;
use crate::model::portfolio::csv::format_iso8601;
use serde::Serialize;

pub const ORDERS_FILE: &str = "paper-orders.csv";
pub const FILLS_FILE: &str = "paper-fills.csv";
pub const TRADES_FILE: &str = "paper-trades.csv";

const ORDERS_HEADER: &str = "order_id,symbol,side,type,quantity,filled,status,created_at";
const FILLS_HEADER: &str =
    "fill_id,order_id,symbol,side,quantity,price,commission,price_source,timestamp";
const TRADES_HEADER: &str = "trade_id,symbol,side,quantity,entry_price,exit_price,gross_pnl,commission,net_pnl,entry_time,exit_time";

/// The three files' text and how many data rows each holds.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PaperExport {
    pub orders: String,
    pub fills: String,
    pub trades: String,
    pub order_rows: usize,
    pub fill_rows: usize,
    pub trade_rows: usize,
}

impl PaperExport {
    /// The files to write, by name.
    pub fn files(&self) -> [(&'static str, &str); 3] {
        [
            (ORDERS_FILE, &self.orders),
            (FILLS_FILE, &self.fills),
            (TRADES_FILE, &self.trades),
        ]
    }

    pub fn row_count(&self) -> usize {
        self.order_rows + self.fill_rows + self.trade_rows
    }

    /// An account that has never traded has nothing worth three files of headers.
    pub fn is_empty(&self) -> bool {
        self.row_count() == 0
    }

    /// "3 orders, 2 fills and 1 closed trade".
    pub fn summary(&self) -> String {
        let count =
            |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
        format!(
            "{}, {} and {}",
            count(self.order_rows, "order", "orders"),
            count(self.fill_rows, "fill", "fills"),
            count(self.trade_rows, "closed trade", "closed trades")
        )
    }
}

/// `value` as the name it is stored under (`"stop_limit"`), which is what the Swift export wrote
/// as the raw value.
fn name<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// A CSV text field: always quoted, quotes doubled.
fn quoted(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

/// Everything `account_id` has done, oldest first within each file.
pub fn export_account(snapshot: &PaperTradingSnapshot, account_id: u64) -> PaperExport {
    let mut orders: Vec<_> = snapshot
        .orders
        .iter()
        .filter(|order| order.account_id == account_id)
        .collect();
    orders.sort_by_key(|order| (order.created_at, order.id));
    let mut fills: Vec<_> = snapshot
        .fills
        .iter()
        .filter(|fill| fill.account_id == account_id)
        .collect();
    fills.sort_by_key(|fill| (fill.timestamp, fill.id));
    let mut trades: Vec<_> = snapshot
        .closed_trades
        .iter()
        .filter(|trade| trade.account_id == account_id)
        .collect();
    trades.sort_by_key(|trade| (trade.exit_timestamp, trade.id));

    let join = |header: &str, rows: Vec<String>| {
        std::iter::once(header.to_owned())
            .chain(rows)
            .collect::<Vec<_>>()
            .join("\n")
            + "\n"
    };
    PaperExport {
        order_rows: orders.len(),
        fill_rows: fills.len(),
        trade_rows: trades.len(),
        orders: join(
            ORDERS_HEADER,
            orders
                .iter()
                .map(|order| {
                    format!(
                        "{},{},{},{},{},{},{},{}",
                        order.id,
                        quoted(&order.instrument.symbol),
                        name(&order.side),
                        name(&order.kind),
                        order.original_quantity,
                        order.filled_quantity,
                        name(&order.status),
                        format_iso8601(order.created_at)
                    )
                })
                .collect(),
        ),
        fills: join(
            FILLS_HEADER,
            fills
                .iter()
                .map(|fill| {
                    format!(
                        "{},{},{},{},{},{},{},{},{}",
                        fill.id,
                        fill.order_id,
                        quoted(&fill.instrument.symbol),
                        name(&fill.side),
                        fill.quantity,
                        fill.price,
                        fill.commission,
                        name(&fill.price_source),
                        format_iso8601(fill.timestamp)
                    )
                })
                .collect(),
        ),
        trades: join(
            TRADES_HEADER,
            trades
                .iter()
                .map(|trade| {
                    format!(
                        "{},{},{},{},{},{},{},{},{},{},{}",
                        trade.id,
                        quoted(&trade.instrument.symbol),
                        name(&trade.side),
                        trade.quantity,
                        trade.entry_price,
                        trade.exit_price,
                        trade.gross_pnl,
                        trade.commission,
                        trade.net_pnl(),
                        format_iso8601(trade.entry_timestamp),
                        format_iso8601(trade.exit_timestamp)
                    )
                })
                .collect(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DataSource;
    use crate::model::paper::{PaperClosedTrade, PaperInstrument, PaperPositionSide};

    fn trade(id: u64, account_id: u64, symbol: &str, exit: i64) -> PaperClosedTrade {
        PaperClosedTrade {
            id,
            account_id,
            instrument: PaperInstrument::chart(symbol, symbol, DataSource::Binance),
            side: PaperPositionSide::Long,
            entry_timestamp: 0,
            exit_timestamp: exit,
            entry_price: 100.0,
            exit_price: 110.5,
            quantity: 2.0,
            gross_pnl: 21.0,
            commission: 1.0,
        }
    }

    #[test]
    fn an_account_with_no_activity_has_nothing_to_export() {
        let snapshot = PaperTradingSnapshot::default();
        let export = export_account(&snapshot, 1);
        assert!(export.is_empty());
        assert_eq!(export.row_count(), 0);
        assert_eq!(export.orders, format!("{ORDERS_HEADER}\n"));
    }

    #[test]
    fn only_the_chosen_accounts_rows_are_written_oldest_first() {
        let snapshot = PaperTradingSnapshot {
            closed_trades: vec![
                trade(2, 1, "ETHUSDT", 2_000),
                trade(1, 1, "BTCUSDT", 1_000),
                trade(3, 9, "SOLUSDT", 500),
            ],
            ..Default::default()
        };
        let export = export_account(&snapshot, 1);
        assert_eq!(export.trade_rows, 2);
        let lines: Vec<&str> = export.trades.lines().collect();
        assert_eq!(lines[0], TRADES_HEADER);
        assert_eq!(
            lines[1],
            "1,\"BTCUSDT\",long,2,100,110.5,21,1,20,1970-01-01T00:00:00.000Z,1970-01-01T00:00:01.000Z"
        );
        assert!(lines[2].starts_with("2,\"ETHUSDT\""));
        assert!(
            !export.trades.contains("SOLUSDT"),
            "another account's trade"
        );
        assert!(!export.is_empty());
    }

    #[test]
    fn symbols_are_quoted_and_quotes_doubled() {
        assert_eq!(quoted("BTC\"X"), "\"BTC\"\"X\"");
    }

    #[test]
    fn the_summary_counts_and_pluralises() {
        let export = PaperExport {
            order_rows: 3,
            fill_rows: 1,
            trade_rows: 0,
            ..Default::default()
        };
        assert_eq!(export.summary(), "3 orders, 1 fill and 0 closed trades");
    }
}
