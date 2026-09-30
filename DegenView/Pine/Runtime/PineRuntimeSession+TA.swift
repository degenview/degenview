import Foundation

/// Glue between `ta.*` calls and the pure `PineTA` indicators: it owns each call site's
/// `PineTA.State` inside the runtime state, so realtime rollback covers it.
extension PineRuntimeSession {
    /// Indicators that take `(source, length)`: `ta.sma`, `ta.ema`, `ta.rsi`…
    func taCall(_ call: PineCall, _ context: inout PineRuntimeContext) throws -> PineRuntimeValue {
        let source = try argument(call, 0, nil, &context)
        let second = try argument(call, 1, nil, &context)
        let length = second.number.flatMap { Int(pine: $0) } ?? 0
        return evaluateTA(
            call.name, source: source, second: second, length: length, site: siteKey(call.site, context))
    }

    func evaluateTA(
        _ name: String, source: PineRuntimeValue, second: PineRuntimeValue = .na, length: Int, site: Int
    ) -> PineRuntimeValue {
        var state = PineTA.State(
            results: working.calls[site] ?? [], inputs: working.callInputs[site] ?? [])
        let result = PineTA.evaluate(name, source: source, second: second, length: length, state: &state)
        working.calls[site] = state.results
        working.callInputs[site] = state.inputs
        return result
    }

    func trueRange(_ bar: KlineData) -> Double {
        PineTA.trueRange(bar, previousClose: working.histories["close"]?.last?.number)
    }

    // MARK: - Calls with their own state shape

    func trueRangeCall(_ call: PineCall, _ context: inout PineRuntimeContext) throws -> PineRuntimeValue {
        .float(trueRange(context.bar))
    }

    /// ATR is the RMA of true range; true range uses the previous close.
    func atrCall(_ call: PineCall, _ context: inout PineRuntimeContext) throws -> PineRuntimeValue {
        let length = try argument(call, 0, nil, &context).number.flatMap { Int(pine: $0) } ?? 0
        return evaluateTA(
            "ta.rma", source: .float(trueRange(context.bar)), length: length,
            site: siteKey(call.site, context))
    }

    func barsSinceCall(_ call: PineCall, _ context: inout PineRuntimeContext) throws -> PineRuntimeValue {
        let site = siteKey(call.site, context)
        let condition = try argument(call, 0, nil, &context)
        let result: PineRuntimeValue
        if condition == .bool(true) {
            result = .int(0)
        } else if let prior = working.calls[site]?.last?.intValue {
            result = .int(prior &+ 1)
        } else {
            result = .na
        }
        working.calls[site] = [result]
        return result
    }

    func cumulativeCall(_ call: PineCall, _ context: inout PineRuntimeContext) throws -> PineRuntimeValue {
        let site = siteKey(call.site, context)
        let total = (working.calls[site]?.last?.number ?? 0) + (try argument(call, 0, nil, &context).number ?? 0)
        working.calls[site] = [.float(total)]
        return .float(total)
    }

    /// `ta.bb(source, length, mult)`: `[basis, upper, lower]`.
    func bollingerCall(_ call: PineCall, _ context: inout PineRuntimeContext) throws -> PineRuntimeValue {
        let site = siteKey(call.site, context)
        let source = try argument(call, 0, nil, &context)
        let length = try argument(call, 1, nil, &context).intValue ?? 0
        let multiplier = try argument(call, 2, nil, &context).number ?? 2
        let basis = evaluateTA("ta.sma", source: source, length: length, site: site)
        let deviation = evaluateTA("ta.stdev", source: source, length: length, site: -(site &+ 1))
        guard let mid = basis.number, let spread = deviation.number else {
            return .tuple([.na, .na, .na])
        }
        return .tuple([.float(mid), .float(mid + multiplier * spread), .float(mid - multiplier * spread)])
    }

    /// `ta.pivothigh` / `ta.pivotlow`. The pivot is the bar `rightbars` back, so it is only
    /// known once that many later bars exist — the value appears on the confirming bar.
    /// The centre must be strictly beyond every left bar and at least as extreme as every
    /// right bar, so a flat top yields one pivot, at its first bar.
    func pivotCall(_ call: PineCall, _ context: inout PineRuntimeContext) throws -> PineRuntimeValue {
        let isHigh = call.name == "ta.pivothigh"
        let positional = call.arguments.filter { $0.name == nil }.count
        let b = try bind(
            call, positional == 2 ? ["leftbars", "rightbars"] : ["source", "leftbars", "rightbars"],
            &context)
        let source = b["source"] ?? market(isHigh ? "high" : "low", context) ?? .na
        guard let left = b["leftbars"].intValue, let right = b["rightbars"].intValue, left >= 0,
            right >= 0
        else { return .na }
        let site = siteKey(call.site, context)
        let needed = left + right + 1
        var window = working.callInputs[site] ?? []
        window.append(source)
        if window.count > needed { window.removeFirst(window.count - needed) }
        working.callInputs[site] = window
        guard window.count == needed, let centre = window[left].number else { return .na }
        func beyond(_ value: PineRuntimeValue, strict: Bool) -> Bool {
            guard let x = value.number else { return false }
            if isHigh { return strict ? centre > x : centre >= x }
            return strict ? centre < x : centre <= x
        }
        guard window[..<left].allSatisfy({ beyond($0, strict: true) }),
            window[(left + 1)...].allSatisfy({ beyond($0, strict: false) })
        else { return .na }
        return .float(centre)
    }
}
