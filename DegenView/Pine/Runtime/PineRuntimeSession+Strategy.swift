import Foundation

extension PineRuntimeSession {
    /// `strategy.entry`, `.order`, `.exit`, `.close`, `.close_all`, `.cancel`, `.cancel_all`;
    /// `strategy.risk.*` limits are accepted and ignored.
    func strategyCall(_ call: PineCall, _ context: inout PineRuntimeContext) throws -> PineRuntimeValue {
        guard isStrategy else {
            throw PineDiagnostic.error(
                "PINE4015", .runtime, "\(call.name) is only available in strategy() scripts.", call.range)
        }
        switch call.name {
        case "strategy.entry", "strategy.order": try placeEntry(call, &context)
        case "strategy.exit": try placeExit(call, &context)
        case "strategy.close": try placeClose(call, &context)
        case "strategy.close_all":
            working.broker.place(.init(kind: .closeAll, id: "Close position order"))
        case "strategy.cancel":
            if let id = try bind(call, ["id"], &context)["id"].textValue { working.broker.cancel(id: id) }
        case "strategy.cancel_all": working.broker.cancelAll()
        default:
            if !call.name.hasPrefix("strategy.risk.") { throw call.unknownFunction }
        }
        return .void
    }

    private func positive(_ value: PineRuntimeValue?) -> Double? {
        guard let n = value?.number, n.isFinite, n > 0 else { return nil }
        return n
    }

    private func placeEntry(_ call: PineCall, _ context: inout PineRuntimeContext) throws {
        let b = try bind(
            call,
            [
                "id", "direction", "qty", "limit", "stop", "oca_name", "oca_type", "comment",
                "alert_message",
            ], &context)
        guard let id = b["id"].textValue, let direction = b["direction"].textValue else { return }
        working.broker.place(
            .init(
                kind: call.name == "strategy.entry" ? .entry : .order, id: id,
                isLong: direction == "strategy.long", quantity: positive(b["qty"]),
                limit: b["limit"]?.number, stop: b["stop"]?.number))
    }

    private func placeExit(_ call: PineCall, _ context: inout PineRuntimeContext) throws {
        let b = try bind(
            call,
            [
                "id", "from_entry", "qty", "qty_percent", "profit", "limit", "loss", "stop",
                "trail_price", "trail_points", "trail_offset",
            ], &context)
        if b["trail_price"]?.number != nil || b["trail_points"]?.number != nil {
            throw PineDiagnostic.error(
                "PINE9005", .unsupported, "strategy.exit trailing stops are not supported yet.", call.range)
        }
        guard let id = b["id"].textValue else { return }
        working.broker.place(
            .init(
                kind: .exit, id: id, quantity: positive(b["qty"]),
                quantityPercent: positive(b["qty_percent"]), limit: b["limit"]?.number,
                stop: b["stop"]?.number, fromEntry: b["from_entry"].textValue,
                profitTicks: positive(b["profit"]), lossTicks: positive(b["loss"])))
    }

    private func placeClose(_ call: PineCall, _ context: inout PineRuntimeContext) throws {
        let b = try bind(call, ["id", "comment", "qty", "qty_percent"], &context)
        guard let id = b["id"].textValue else { return }
        working.broker.place(
            .init(
                kind: .close, id: id, quantity: positive(b["qty"]),
                quantityPercent: positive(b["qty_percent"])))
    }

    /// `strategy.*` identifiers (`strategy.position_size`, `strategy.equity`…).
    func strategyValue(_ name: String, _ bar: KlineData) -> PineRuntimeValue? {
        let broker = working.broker
        switch name {
        case "strategy.position_size": return .float(broker.positionSize)
        case "strategy.position_avg_price":
            return broker.positionSize == 0 ? .na : .float(broker.averagePrice)
        case "strategy.equity": return .float(broker.equity(at: bar.closePrice))
        case "strategy.netprofit": return .float(broker.netProfit)
        case "strategy.openprofit": return .float(broker.openProfit(at: bar.closePrice))
        case "strategy.initial_capital": return .float(broker.settings.initialCapital)
        case "strategy.closedtrades": return .int(broker.closedTrades.count)
        case "strategy.opentrades": return .int(broker.openTrades.count)
        case "strategy.wintrades": return .int(broker.closedTrades.filter { $0.profit > 0 }.count)
        case "strategy.losstrades": return .int(broker.closedTrades.filter { $0.profit < 0 }.count)
        case "strategy.grossprofit":
            return .float(broker.closedTrades.filter { $0.profit > 0 }.reduce(0) { $0 + $1.profit })
        case "strategy.grossloss":
            return .float(-broker.closedTrades.filter { $0.profit < 0 }.reduce(0) { $0 + $1.profit })
        default: return nil
        }
    }
}
