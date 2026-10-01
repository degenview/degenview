import Foundation

/// Identifiers that read the current bar, the symbol or the timeframe.
extension PineRuntimeSession {
    func market(_ name: String, _ context: PineRuntimeContext) -> PineRuntimeValue? {
        let bar = context.bar
        switch name {
        case "open": return .float(bar.openPrice)
        case "high": return .float(bar.highPrice)
        case "low": return .float(bar.lowPrice)
        case "close": return .float(bar.closePrice)
        case "volume": return .float(bar.volume)
        case "hl2": return .float((bar.highPrice + bar.lowPrice) / 2)
        case "hlc3": return .float((bar.highPrice + bar.lowPrice + bar.closePrice) / 3)
        case "ohlc4": return .float((bar.openPrice + bar.highPrice + bar.lowPrice + bar.closePrice) / 4)
        case "time": return .int(PineTime.milliseconds(bar.openTime))
        case "time_close":
            return .int(PineTime.milliseconds(bar.openTime) + (Int(pine: barSeconds * 1000) ?? 0))
        case "bar_index": return .int(working.barIndex)
        case "last_bar_index": return .int(lastBarIndex)
        case "last_bar_time": return lastBarTime.map { .int(PineTime.milliseconds($0)) } ?? .na
        case "timenow": return .int(PineTime.milliseconds(Date()))
        case "ta.tr": return .float(trueRange(bar))
        case "year", "month", "dayofmonth", "hour", "minute", "second", "dayofweek":
            return PineTime.part(name, milliseconds: PineTime.milliseconds(bar.openTime))
        default: return symbolValue(name) ?? namespaced(name, bar)
        }
    }

    private func symbolValue(_ name: String) -> PineRuntimeValue? {
        switch name {
        case "syminfo.mintick": .float(mintick)
        case "syminfo.ticker": .string(symbol.ticker)
        case "syminfo.tickerid": .string(symbol.tickerID)
        case "syminfo.currency": .string(symbol.currency)
        case "syminfo.type": .string(symbol.type)
        default: nil
        }
    }

    private func namespaced(_ name: String, _ bar: KlineData) -> PineRuntimeValue? {
        if name.hasPrefix("strategy.") { return strategyValue(name, bar) }
        if name.hasPrefix("timeframe.") { return PineTime.timeframe(name, barSeconds: barSeconds) }
        return nil
    }
}
