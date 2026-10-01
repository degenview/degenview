import Foundation

/// Glue between `ta.*` calls and the pure `PineTA` indicators: it owns each call site's
/// `PineTA.State` inside the runtime state, so realtime rollback covers it.
extension PineRuntimeSession {
    /// Indicators that take `(source, length)`: `ta.sma`, `ta.ema`, `ta.rsi`…
    /// The series `ta.highest(length)` and friends read when called with a length alone.
    private static let defaultSources = [
        "ta.highest": "high", "ta.lowest": "low", "ta.highestbars": "high", "ta.lowestbars": "low",
    ]

    func taCall(_ call: PineCall, _ context: inout PineRuntimeContext) throws -> PineRuntimeValue {
        guard PineTA.supported.contains(call.name) else { throw call.unknownFunction }
        let source: PineRuntimeValue
        let second: PineRuntimeValue
        if let series = Self.defaultSources[call.name], call.arguments.count == 1,
            call.arguments[0].name == nil
        {
            source = market(series, context) ?? .na
            second = try argument(call, 0, nil, &context)
        } else {
            source = try argument(call, 0, nil, &context)
            second = try argument(call, 1, nil, &context)
        }
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

    /// `ta.correlation(source1, source2, length)`: the Pearson correlation over the last `length` pairs.
    func correlationCall(_ call: PineCall, _ context: inout PineRuntimeContext) throws -> PineRuntimeValue {
        let site = siteKey(call.site, context)
        let first = try argument(call, 0, nil, &context)
        let second = try argument(call, 1, nil, &context)
        let length = try argument(call, 2, nil, &context).intValue ?? 0
        var pairs = working.calls[site] ?? []
        pairs.append(.tuple([first, second]))
        working.calls[site] = pairs
        guard length > 0, pairs.count >= length else { return .na }
        var xs: [Double] = []
        var ys: [Double] = []
        for pair in pairs.suffix(length) {
            guard case .tuple(let items) = pair, let x = items[0].number, let y = items[1].number else {
                return .na
            }
            xs.append(x)
            ys.append(y)
        }
        let meanX = xs.reduce(0, +) / Double(length)
        let meanY = ys.reduce(0, +) / Double(length)
        let covariance = zip(xs, ys).reduce(0) { $0 + ($1.0 - meanX) * ($1.1 - meanY) }
        let spread =
            (xs.reduce(0) { $0 + ($1 - meanX) * ($1 - meanX) } * ys.reduce(0) { $0 + ($1 - meanY) * ($1 - meanY) })
            .squareRoot()
        return spread == 0 ? .na : .float(covariance / spread)
    }

    /// `ta.vwap(source, anchor, stdev_mult)`: volume weighted average price, restarting each UTC day unless the
    /// script passes its own `anchor`. With a multiplier it is `[vwap, upper, lower]` using the volume weighted
    /// standard deviation of the source.
    func vwapCall(_ call: PineCall, _ context: inout PineRuntimeContext) throws -> PineRuntimeValue {
        let site = siteKey(call.site, context)
        let b = try bind(call, ["source", "anchor", "stdev_mult"], &context)
        let price = b["source"]?.number ?? context.bar.closePrice
        let volume = context.bar.volume
        let day = Int(context.bar.openTime.timeIntervalSince1970 / 86_400)
        var state = working.calls[site]?.last.flatMap { value -> [Double]? in
            if case .tuple(let items) = value { return items.compactMap(\.number) }
            return nil
        } ?? [0, 0, 0, Double(day)]
        let reset = b["anchor"]?.bool ?? (Int(state[3]) != day)
        if reset { state = [0, 0, 0, Double(day)] }
        state[0] += price * volume
        state[1] += volume
        state[2] += price * price * volume
        state[3] = Double(day)
        working.calls[site] = [.tuple(state.map(PineRuntimeValue.float))]
        guard state[1] > 0 else { return b["stdev_mult"] == nil ? .na : .tuple([.na, .na, .na]) }
        let vwap = state[0] / state[1]
        guard let multiplier = b["stdev_mult"]?.number else { return .float(vwap) }
        let deviation = max(0, state[2] / state[1] - vwap * vwap).squareRoot()
        return .tuple([
            .float(vwap), .float(vwap + multiplier * deviation), .float(vwap - multiplier * deviation),
        ])
    }

    /// `ta.dmi(diLength, adxSmoothing)`: `[+DI, −DI, ADX]`, from the smoothed directional movement and true range.
    func dmiCall(_ call: PineCall, _ context: inout PineRuntimeContext) throws -> PineRuntimeValue {
        let site = siteKey(call.site, context)
        let diLength = try argument(call, 0, "diLength", &context).intValue ?? 0
        let adxSmoothing = try argument(call, 1, "adxSmoothing", &context).intValue ?? 0
        let bar = context.bar
        func smoothed(_ key: Int, _ value: PineRuntimeValue, _ length: Int) -> PineRuntimeValue {
            evaluateTA("ta.rma", source: value, length: length, site: site &* 16 &+ key)
        }
        var plusMove: PineRuntimeValue = .na
        var minusMove: PineRuntimeValue = .na
        if let previousHigh = working.histories["high"]?.last?.number,
            let previousLow = working.histories["low"]?.last?.number
        {
            let up = bar.highPrice - previousHigh
            let down = previousLow - bar.lowPrice
            plusMove = .float(up > down && up > 0 ? up : 0)
            minusMove = .float(down > up && down > 0 ? down : 0)
        }
        let range = smoothed(1, .float(trueRange(bar)), diLength)
        let plusSmoothed = smoothed(2, plusMove, diLength)
        let minusSmoothed = smoothed(3, minusMove, diLength)
        var plus: PineRuntimeValue = .na
        var minus: PineRuntimeValue = .na
        var strength: PineRuntimeValue = .na
        if let r = range.number, r != 0, let p = plusSmoothed.number, let m = minusSmoothed.number {
            plus = .float(100 * p / r)
            minus = .float(100 * m / r)
            let total = 100 * p / r + 100 * m / r
            strength = .float(abs(100 * p / r - 100 * m / r) / (total == 0 ? 1 : total))
        }
        let adx = smoothed(4, strength, adxSmoothing).number.map { PineRuntimeValue.float(100 * $0) } ?? .na
        return .tuple([plus, minus, adx])
    }

    /// `ta.sar(start, inc, max)`: Wilder's parabolic SAR, following the equivalent Pine code in the reference
    /// manual. The state is `[sar, extreme point, acceleration, 1 when the SAR is below price]`.
    func sarCall(_ call: PineCall, _ context: inout PineRuntimeContext) throws -> PineRuntimeValue {
        let site = siteKey(call.site, context)
        let b = try bind(call, ["start", "inc", "max"], &context)
        guard let start = b["start"]?.number, let step = b["inc"]?.number, let limit = b["max"]?.number,
            let highs = working.histories["high"], let lows = working.histories["low"],
            let closes = working.histories["close"], let lastHigh = highs.last?.number,
            let lastLow = lows.last?.number, let lastClose = closes.last?.number
        else { return .na }
        let bar = context.bar
        let barIndex = working.barIndex
        var state: [Double]
        var firstTrendBar = false
        if barIndex == 1 {
            let below = bar.closePrice > lastClose
            state = [below ? lastLow : lastHigh, below ? bar.highPrice : bar.lowPrice, start, below ? 1 : 0]
            firstTrendBar = true
        } else if case .tuple(let items)? = working.calls[site]?.last, items.count == 4 {
            state = items.compactMap(\.number)
        } else {
            return .na
        }
        var (sar, extreme, acceleration) = (state[0], state[1], state[2])
        var below = state[3] == 1
        sar += acceleration * (extreme - sar)
        if below, sar > bar.lowPrice {
            firstTrendBar = true
            below = false
            sar = max(bar.highPrice, extreme)
            extreme = bar.lowPrice
            acceleration = start
        } else if !below, sar < bar.highPrice {
            firstTrendBar = true
            below = true
            sar = min(bar.lowPrice, extreme)
            extreme = bar.highPrice
            acceleration = start
        }
        if !firstTrendBar {
            if below, bar.highPrice > extreme {
                extreme = bar.highPrice
                acceleration = min(acceleration + step, limit)
            } else if !below, bar.lowPrice < extreme {
                extreme = bar.lowPrice
                acceleration = min(acceleration + step, limit)
            }
        }
        let olderHigh = highs.count > 1 ? highs[highs.count - 2].number : nil
        let olderLow = lows.count > 1 ? lows[lows.count - 2].number : nil
        if below {
            sar = min(sar, lastLow)
            if barIndex > 1, let olderLow { sar = min(sar, olderLow) }
        } else {
            sar = max(sar, lastHigh)
            if barIndex > 1, let olderHigh { sar = max(sar, olderHigh) }
        }
        working.calls[site] = [.tuple([sar, extreme, acceleration, below ? 1 : 0].map(PineRuntimeValue.float))]
        return .float(sar)
    }
}

