import Foundation

/// `request.security`.
///
/// For the chart's own symbol the expression runs against a higher-timeframe series built by folding the
/// chart's bars, in a state of its own (`PineSecuritySite.state`) swapped in for `working` while it
/// evaluates. If the session's `PineSecurityDataProvider` has that timeframe, its candles from before the
/// chart's first bar are fed in first, so the series has the history it would have on TradingView. Another
/// symbol is read from the candles the provider supplies, and is an error when it supplies none (see
/// `PineRuntimeSession+ForeignSecurity`).
/// Semantics for the chart's own symbol, as implemented (not checked against TradingView):
/// - history and confirmed bars: the value on the last higher-timeframe bar completed by the close of
///   the chart bar; a chart bar that closes its bucket completes it and returns its own value;
/// - realtime bars: the developing higher-timeframe bar;
/// - `lookahead_on`: the developing bar always, which equals Pine for the `expr[1]` idiom but never
///   looks ahead;
/// - `gaps_on`: `na` except on the chart bar where a new value arrives.
extension PineRuntimeSession {
    struct SecurityRequest {
        var interval: TimeInterval
        var expression: PineExpression
        var lookaheadOn: Bool
        var gapsOn: Bool
        /// The candles of another symbol's series; nil for the chart's own.
        var foreign: [KlineData]?
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
        let request: SecurityRequest
        switch try securityRequest(call, &context) {
        case .serve(let served): request = served
        case .unserved(let expression):
            // An invalid symbol or timeframe the script asked to ignore: na, shaped like the expression.
            if case .tuple(let items, _) = expression { return .tuple(items.map { _ in .na }) }
            return .na
        }
        if let candles = request.foreign { return try serveForeign(request, candles, call, &context) }
        return try serveSecurity(request, call, &context)
    }

    /// The expression on the higher-timeframe bar the chart bar belongs to, through the call site's own state.
    private func serveSecurity(
        _ request: SecurityRequest, _ call: PineCall, _ context: inout PineRuntimeContext
    ) throws -> PineRuntimeValue {
        let key = siteKey(call.site, context)
        var site = working.securities[key] ?? PineSecuritySite()
        let bar = context.bar
        if site.processedBar == bar.openTime { return site.barResult }
        if !site.isWarmedUp { try warmUp(&site, request, before: bar.openTime) }

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
    private enum SecurityOutcome {
        case serve(SecurityRequest)
        /// The symbol or timeframe cannot be served and the script passed `ignore_invalid_*`.
        case unserved(PineExpression)
    }

    private func securityRequest(
        _ call: PineCall, _ context: inout PineRuntimeContext
    ) throws -> SecurityOutcome {
        let (values, expression) = try securityArguments(call, Self.securityParameters, &context)
        guard let expression else {
            throw PineDiagnostic.error(
                "PINE4024", .runtime, "request.security needs a symbol, a timeframe and an expression.",
                call.range)
        }
        let ignoreSymbol = values["ignore_invalid_symbol"] == .bool(true)
        guard case .string(let name)? = values["symbol"] else {
            if ignoreSymbol { return .unserved(expression) }
            throw unservedSymbol("", call.range)
        }
        let isChartSymbol = name.isEmpty || name == symbol.tickerID
        if !isChartSymbol, securityData == nil {
            if ignoreSymbol { return .unserved(expression) }
            throw unservedSymbol(name, call.range)
        }
        let interval: TimeInterval
        do {
            interval = try securityInterval(values["timeframe"], allowFiner: !isChartSymbol, call.range)
        } catch {
            if values["ignore_invalid_timeframe"] == .bool(true) { return .unserved(expression) }
            throw error
        }
        var candles: [KlineData]?
        if !isChartSymbol {
            candles = series(PineSecurityKey(symbol: name, interval: interval))
            if candles == nil {
                if ignoreSymbol { return .unserved(expression) }
                throw unservedSymbol(name, call.range)
            }
        }
        return .serve(
            SecurityRequest(
                interval: interval, expression: expression,
                lookaheadOn: values["lookahead"] == .string("barmerge.lookahead_on"),
                gapsOn: values["gaps"] == .string("barmerge.gaps_on"), foreign: candles))
    }

    /// The provider's candles for `key`, asked once per run.
    func series(_ key: PineSecurityKey) -> [KlineData]? {
        if let known = securitySeries[key] { return known }
        let answer = securityData?.candles(for: key)
        securitySeries[key] = .some(answer)
        return answer
    }

    private func unservedSymbol(_ name: String, _ range: PineSourceRange) -> PineDiagnostic {
        .error(
            "PINE4022", .runtime,
            name.isEmpty || securityData == nil
                ? "request.security only supports the chart's own symbol (syminfo.tickerid) here."
                : "No data is available for symbol '\(name)'.", range)
    }

    /// Evaluates the candles the provider holds for the chart's own symbol that closed before the chart's first
    /// bar, so the expression starts with the history TradingView would have given it.
    private func warmUp(_ site: inout PineSecuritySite, _ request: SecurityRequest, before first: Date) throws {
        site.isWarmedUp = true
        guard let candles = series(PineSecurityKey(symbol: symbol.tickerID, interval: request.interval)) else {
            return
        }
        for candle in candles {
            guard KlineData.bucketEnd(after: candle.openTime, interval: request.interval) <= first else { break }
            site.lastResult = try evaluateHigherTimeframe(&site, candle, request, commit: true)
        }
    }

    /// The call's arguments by parameter name. `expression` stays unevaluated: it runs on another series.
    private func securityArguments(
        _ call: PineCall, _ parameters: [String], _ context: inout PineRuntimeContext
    ) throws -> ([String: PineRuntimeValue], PineExpression?) {
        var values: [String: PineRuntimeValue] = [:]
        var expression: PineExpression?
        var position = 0
        for argument in call.arguments {
            var name = argument.name
            if name == nil {
                if position < parameters.count { name = parameters[position] }
                position += 1
            }
            guard let name else { continue }
            if name == "expression" {
                expression = argument.value
            } else {
                values[name] = try eval(argument.value, &context)
            }
        }
        return (values, expression)
    }

    // MARK: - Lower timeframes

    private static let lowerTimeframeParameters = [
        "symbol", "timeframe", "expression", "ignore_invalid_symbol", "currency",
        "ignore_invalid_timeframe", "calc_bars_count",
    ]

    /// `request.security_lower_tf`: the values of an expression on each intrabar of the chart bar. The engine
    /// only has the chart's own bars, so there is no intrabar data: every array is empty, which is what
    /// Pine returns when a timeframe cannot be served. A tuple expression gives a tuple of empty arrays.
    /// An invalid symbol or timeframe is an error unless the script passed `ignore_invalid_*`, as in Pine.
    func securityLowerTimeframeCall(
        _ call: PineCall, _ context: inout PineRuntimeContext
    ) throws -> PineRuntimeValue {
        if securityGlobals != nil {
            throw PineDiagnostic.error(
                "PINE4023", .runtime, "request.security_lower_tf cannot be called inside request.security.",
                call.range)
        }
        let (values, expression) = try securityArguments(call, Self.lowerTimeframeParameters, &context)
        guard let expression else {
            throw PineDiagnostic.error(
                "PINE4024", .runtime,
                "request.security_lower_tf needs a symbol, a timeframe and an expression.", call.range)
        }
        if case .string(let text)? = values["symbol"], !(text.isEmpty || text == symbol.tickerID),
            values["ignore_invalid_symbol"] != .bool(true)
        {
            throw PineDiagnostic.error(
                "PINE4022", .runtime,
                "request.security_lower_tf only supports the chart's own symbol in this release.", call.range)
        }
        let seconds = values["timeframe"].textValue.flatMap(PineTime.seconds(ofTimeframe:))
        let usable = seconds.map { $0 <= barSeconds } ?? false
        if !usable, barSeconds > 0, values["ignore_invalid_timeframe"] != .bool(true) {
            throw PineDiagnostic.error(
                "PINE4021", .runtime,
                "request.security_lower_tf needs a timeframe no longer than the chart's.", call.range)
        }
        func array(_ items: [PineRuntimeValue]) -> PineRuntimeValue {
            let id = allocate()
            working.arrays[id] = items
            return .ref(.array, id)
        }
        // The chart's own timeframe: each chart bar is its own single intrabar. A lower one has no data here.
        var intrabars: PineRuntimeValue?
        if let seconds, seconds == barSeconds {
            intrabars = try serveSecurity(
                SecurityRequest(interval: seconds, expression: expression, lookaheadOn: false, gapsOn: false),
                call, &context)
        }
        if case .tuple(let items, _) = expression {
            guard case .tuple(let values)? = intrabars else { return .tuple(items.map { _ in array([]) }) }
            return .tuple(values.map { array([$0]) })
        }
        return array(intrabars.map { [$0] } ?? [])
    }

    private func securityInterval(
        _ value: PineRuntimeValue?, allowFiner: Bool, _ range: PineSourceRange
    ) throws -> TimeInterval {
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
        if interval < barSeconds, !allowFiner {
            throw PineDiagnostic.error(
                "PINE4021", .runtime,
                "request.security cannot use a timeframe finer than the chart's; lower timeframes are not supported.",
                range)
        }
        return interval
    }

    /// Runs the expression on `candle` in the site's own state. A commit keeps that state and
    /// appends the candle to its histories; otherwise the state is thrown away, like a realtime tick.
    func evaluateHigherTimeframe(
        _ site: inout PineSecuritySite, _ candle: KlineData, _ request: SecurityRequest, commit: Bool
    ) throws -> PineRuntimeValue {
        var chart = working
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
        let evaluated = try eval(request.expression, &context)
        // A returned collection or object lives in this site's state; the chart gets a copy of its own.
        var exporter = PineReferenceExport(from: working)
        let value = exporter.export(evaluated, into: &chart)
        if commit {
            commitHistories(candle)
            site.state = working
        }
        return value
    }
}
