import Foundation

/// `request.security` for the chart's own symbol.
///
/// The expression runs against a higher-timeframe series built by folding the chart's bars, in a
/// state of its own (`PineSecuritySite.state`) swapped in for `working` while it evaluates.
/// Semantics, as implemented (not checked against TradingView):
/// - history and confirmed bars: the value on the last higher-timeframe bar completed by the close of
///   the chart bar; a chart bar that closes its bucket completes it and returns its own value;
/// - realtime bars: the developing higher-timeframe bar;
/// - `lookahead_on`: the developing bar always, which equals Pine for the `expr[1]` idiom but never
///   looks ahead;
/// - `gaps_on`: `na` except on the chart bar where a new value arrives.
extension PineRuntimeSession {
    private struct SecurityRequest {
        var interval: TimeInterval
        var expression: PineExpression
        var lookaheadOn: Bool
        var gapsOn: Bool
    }

    private static let securityParameters = [
        "symbol", "timeframe", "expression", "gaps", "lookahead", "ignore_invalid_symbol", "currency",
        "ignore_invalid_timeframe", "calc_bars_count",
    ]

    func securityCall(_ call: PineCall, _ context: inout PineRuntimeContext) throws -> PineRuntimeValue {
        if securityGlobals != nil {
            throw PineDiagnostic.error(
                "PINE4023", .runtime, "request.security cannot be called inside another request.security.",
                call.range)
        }
        let request = try securityRequest(call, &context)
        let key = siteKey(call.site, context)
        var site = working.securities[key] ?? PineSecuritySite()
        let bar = context.bar
        if site.processedBar == bar.openTime { return site.barResult }

        let start = KlineData.bucketStart(of: bar.openTime, interval: request.interval)
        var fresh = false
        if site.bucket != start {
            // The chart skipped the bucket's last bar, so complete it now.
            if site.hasPendingBars, let pending = site.accumulated {
                site.lastResult = try evaluateHigherTimeframe(&site, pending, request, commit: true)
                fresh = true
            }
            site.bucket = start
            site.accumulated = nil
            site.hasPendingBars = false
        }
        let developing = ([site.accumulated].compactMap { $0 } + [bar]).folded(into: request.interval)[0]
        let end = KlineData.bucketEnd(after: start, interval: request.interval)
        let closesBucket = bar.openTime.addingTimeInterval(barSeconds) >= end
        let confirmed = context.flags.isConfirmed

        var result: PineRuntimeValue
        if closesBucket && confirmed {
            result = try evaluateHigherTimeframe(&site, developing, request, commit: true)
            site.lastResult = result
            site.accumulated = nil
            site.hasPendingBars = false
            fresh = true
        } else {
            if confirmed {
                site.accumulated = developing
                site.hasPendingBars = true
            }
            if request.lookaheadOn || context.flags.isRealtime {
                result = try evaluateHigherTimeframe(&site, developing, request, commit: false)
            } else {
                result = site.lastResult
            }
        }
        if request.gapsOn && !fresh { result = .na }
        site.processedBar = bar.openTime
        site.barResult = result
        working.securities[key] = site
        return result
    }

    /// Reads the call's arguments. `expression` is kept unevaluated: it runs on the higher timeframe.
    private func securityRequest(
        _ call: PineCall, _ context: inout PineRuntimeContext
    ) throws -> SecurityRequest {
        var values: [String: PineRuntimeValue] = [:]
        var expression: PineExpression?
        var position = 0
        for argument in call.arguments {
            var name = argument.name
            if name == nil {
                if position < Self.securityParameters.count { name = Self.securityParameters[position] }
                position += 1
            }
            guard let name else { continue }
            if name == "expression" {
                expression = argument.value
            } else {
                values[name] = try eval(argument.value, &context)
            }
        }
        guard let expression else {
            throw PineDiagnostic.error(
                "PINE4024", .runtime, "request.security needs a symbol, a timeframe and an expression.",
                call.range)
        }
        try requireChartSymbol(values["symbol"], call.range)
        return SecurityRequest(
            interval: try securityInterval(values["timeframe"], call.range), expression: expression,
            lookaheadOn: values["lookahead"] == .string("barmerge.lookahead_on"),
            gapsOn: values["gaps"] == .string("barmerge.gaps_on"))
    }

    private func requireChartSymbol(_ value: PineRuntimeValue?, _ range: PineSourceRange) throws {
        if case .string(let text)? = value, text.isEmpty || text == symbol.tickerID { return }
        throw PineDiagnostic.error(
            "PINE4022", .runtime,
            "request.security only supports the chart's own symbol (syminfo.tickerid) in this release.", range)
    }

    private func securityInterval(_ value: PineRuntimeValue?, _ range: PineSourceRange) throws -> TimeInterval {
        let interval: TimeInterval
        switch value {
        case .string(let text)? where !text.isEmpty:
            guard let seconds = PineTime.seconds(ofTimeframe: text) else {
                throw PineDiagnostic.error(
                    "PINE4021", .runtime, "request.security needs a timeframe such as \"60\" or \"1D\".", range)
            }
            interval = seconds
        default: interval = barSeconds
        }
        guard interval > 0 else {
            throw PineDiagnostic.error(
                "PINE4021", .runtime, "The chart's bar length is not known yet.", range)
        }
        if interval < barSeconds {
            throw PineDiagnostic.error(
                "PINE4021", .runtime,
                "request.security cannot use a timeframe finer than the chart's; lower timeframes are not supported.",
                range)
        }
        return interval
    }

    /// Runs the expression on `candle` in the site's own state. A commit keeps that state and
    /// appends the candle to its histories; otherwise the state is thrown away, like a realtime tick.
    private func evaluateHigherTimeframe(
        _ site: inout PineSecuritySite, _ candle: KlineData, _ request: SecurityRequest, commit: Bool
    ) throws -> PineRuntimeValue {
        let chart = working
        let chartBarSeconds = barSeconds
        securityGlobals = chart.variables
        working = site.state
        working.instructions = 0
        working.barIndex = site.state.barIndex + 1
        barSeconds = request.interval
        defer {
            working = chart
            barSeconds = chartBarSeconds
            securityGlobals = nil
        }
        let flags = PineBarFlags(
            isFirst: working.barIndex == 0, isLast: false, isHistory: commit, isRealtime: !commit,
            isNew: true, isConfirmed: commit, isLastConfirmedHistory: false)
        var context = PineRuntimeContext(bar: candle, flags: flags)
        let value = try eval(request.expression, &context)
        if commit {
            commitHistories(candle)
            site.state = working
        }
        return value
    }
}
