import Foundation

enum PaperTradingCSVExporter {
    static func export(snapshot: PaperTradingSnapshot, accountID: UUID?, to directory: URL) {
        guard let accountID else { return }
        let iso = ISO8601DateFormatter()
        let orders =
            (["order_id,symbol,side,type,quantity,filled,status,created_at"]
            + snapshot.orders.filter { $0.accountID == accountID }.map {
                "\($0.id.uuidString),\(csv($0.instrument.symbol)),\($0.side.rawValue),\($0.type.rawValue),\($0.originalQuantity),\($0.filledQuantity),\($0.status.rawValue),\(iso.string(from: $0.createdAt))"
            }).joined(separator: "\n")
        let fills =
            (["fill_id,order_id,symbol,side,quantity,price,commission,price_source,timestamp"]
            + snapshot.fills.filter { $0.accountID == accountID }.map {
                "\($0.id.uuidString),\($0.orderID.uuidString),\(csv($0.instrument.symbol)),\($0.side.rawValue),\($0.quantity),\($0.price),\($0.commission),\($0.priceSource.rawValue),\(iso.string(from: $0.timestamp))"
            }).joined(separator: "\n")
        let trades =
            (["trade_id,symbol,side,quantity,entry_price,exit_price,gross_pnl,commission,net_pnl,entry_time,exit_time"]
            + snapshot.closedTrades.filter { $0.accountID == accountID }.map {
                "\($0.id.uuidString),\(csv($0.instrument.symbol)),\($0.side.rawValue),\($0.quantity),\($0.entryPrice),\($0.exitPrice),\($0.grossPnL),\($0.commission),\($0.netPnL),\(iso.string(from: $0.entryTimestamp)),\(iso.string(from: $0.exitTimestamp))"
            }).joined(separator: "\n")
        try? orders.write(to: directory.appendingPathComponent("paper-orders.csv"), atomically: true, encoding: .utf8)
        try? fills.write(to: directory.appendingPathComponent("paper-fills.csv"), atomically: true, encoding: .utf8)
        try? trades.write(to: directory.appendingPathComponent("paper-trades.csv"), atomically: true, encoding: .utf8)
    }
    private static func csv(_ value: String) -> String { "\"\(value.replacingOccurrences(of: "\"", with: "\"\""))\"" }
}
