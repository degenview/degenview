import Foundation

/// `request.security` for a symbol the chart does not carry, read from the candles a `PineSecurityDataProvider`
/// supplied (a snapshot: it does not tick).
///
/// Semantics, as implemented (not checked against TradingView):
/// - a confirmed chart bar completes every candle of the series that has closed by the chart bar's close, in
///   order, each through the site's own state, and answers the value on the last one; with `gaps_on` the answer
///   is `na` on chart bars that completed nothing;
/// - `lookahead_on`, and any realtime bar, answer the value on the candle that contains the chart bar's open,
///   whole as supplied. On history that reads a candle that has not closed yet, which is what TradingView does;
/// - a series finer than the chart's is allowed: several candles complete per chart bar and the last one answers.
extension PineRuntimeSession {
    func serveForeign(
        _ request: SecurityRequest, _ candles: [KlineData], _ call: PineCall,
        _ context: inout PineRuntimeContext
    ) throws -> PineRuntimeValue {
        let key = siteKey(call.site, context)
        var site = working.securities[key] ?? PineSecuritySite()
        let bar = context.bar
        if site.processedBar == bar.openTime { return site.barResult }

        var fresh = false
        if context.flags.isConfirmed {
            let chartClose = bar.openTime.addingTimeInterval(barSeconds)
            while site.completedCandles < candles.count {
                let candle = candles[site.completedCandles]
                guard KlineData.bucketEnd(after: candle.openTime, interval: request.interval) <= chartClose
                else { break }
                site.lastResult = try evaluateHigherTimeframe(&site, candle, request, commit: true)
                site.completedCandles += 1
                fresh = true
            }
        }
        var result = site.lastResult
        if request.lookaheadOn || context.flags.isRealtime,
            let index = Self.lastIndex(opening: bar.openTime, in: candles), index >= site.completedCandles
        {
            result = try evaluateHigherTimeframe(&site, candles[index], request, commit: false)
        }
        if request.gapsOn && !fresh { result = .na }
        site.processedBar = bar.openTime
        site.barResult = result
        working.securities[key] = site
        return result
    }

    /// The last candle that opens at or before `time`, found by bisection; nil when all open later.
    private static func lastIndex(opening time: Date, in candles: [KlineData]) -> Int? {
        var low = 0
        var high = candles.count
        while low < high {
            let middle = (low + high) / 2
            if candles[middle].openTime <= time { low = middle + 1 } else { high = middle }
        }
        return low == 0 ? nil : low - 1
    }
}
