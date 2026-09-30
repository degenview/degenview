import Foundation

/// `ta.*` indicators over the per-call-site history the session keeps. Pure: everything an
/// indicator needs is in `State`.
enum PineTA {
    /// What one call site remembers between bars.
    struct State {
        /// One entry per evaluation. Usually the indicator's value; for `ta.cross*` the
        /// `[source, second]` pair, for `ta.macd` the `[macd, signal, histogram]` tuple.
        var results: [PineRuntimeValue] = []
        /// The source series, one entry per evaluation, `na` included.
        var inputs: [PineRuntimeValue] = []
    }

    private static let macdFast = 12
    private static let macdSlow = 26
    private static let macdSignal = 9

    /// Evaluates `name` for the current bar and records the bar in `state`.
    static func evaluate(
        _ name: String, source: PineRuntimeValue, second: PineRuntimeValue = .na, length: Int,
        state: inout State
    ) -> PineRuntimeValue {
        state.inputs.append(source)
        let result: PineRuntimeValue
        switch name {
        case "ta.sma": result = sma(state.inputs, length)
        case "ta.ema", "ta.rma": result = exponential(name, source, state, length)
        case "ta.highest": result = extremum(state.inputs, length, pick: { $0.max() })
        case "ta.lowest": result = extremum(state.inputs, length, pick: { $0.min() })
        case "ta.change": result = change(state.inputs, length > 0 ? length : 1)
        case "ta.wma": result = wma(state.inputs, length)
        case "ta.stdev": result = stdev(state.inputs, length)
        case "ta.rising": result = trend(state.inputs, length, rising: true)
        case "ta.falling": result = trend(state.inputs, length, rising: false)
        case "ta.mom", "ta.roc": result = momentum(state.inputs, length, isRate: name == "ta.roc")
        case "ta.rsi": result = rsi(state.inputs, length)
        case "ta.macd": result = macd(state)
        case "ta.cross", "ta.crossover", "ta.crossunder":
            let crossed = cross(name, source, second, prior: state.results.last)
            state.results.append(.tuple([source, second]))
            return crossed
        default: result = .na
        }
        state.results.append(result)
        return result
    }

    /// True range: the widest of the bar's range and its gaps from the previous close.
    static func trueRange(_ bar: KlineData, previousClose: Double?) -> Double {
        max(
            bar.highPrice - bar.lowPrice,
            max(
                previousClose.map { abs(bar.highPrice - $0) } ?? 0,
                previousClose.map { abs(bar.lowPrice - $0) } ?? 0))
    }

    // MARK: - Windows

    /// The last `count` non-`na` inputs, oldest first, or nil when fewer exist.
    private static func lastValid(_ inputs: [PineRuntimeValue], _ count: Int) -> [Double]? {
        guard count > 0, count <= inputs.count else { return nil }
        var found: [Double] = []
        found.reserveCapacity(count)
        var i = inputs.count - 1
        while i >= 0, found.count < count {
            if let n = inputs[i].number { found.append(n) }
            i -= 1
        }
        return found.count == count ? Array(found.reversed()) : nil
    }

    /// The last `count` inputs as numbers, or nil when there are fewer or any is `na`.
    private static func lastWindow(_ inputs: [PineRuntimeValue], _ count: Int) -> [Double]? {
        guard count > 0, count <= inputs.count else { return nil }
        let numbers = inputs.suffix(count).compactMap(\.number)
        return numbers.count == count ? numbers : nil
    }

    // MARK: - Indicators

    private static func mean(_ values: [Double]?, _ length: Int) -> PineRuntimeValue {
        values.map { .float($0.reduce(0, +) / Double(length)) } ?? .na
    }

    private static func sma(_ inputs: [PineRuntimeValue], _ length: Int) -> PineRuntimeValue {
        mean(lastValid(inputs, length), length)
    }

    /// EMA and RMA differ only in alpha. Both seed from the simple average of the first
    /// `length` values.
    private static func exponential(
        _ name: String, _ source: PineRuntimeValue, _ state: State, _ length: Int
    ) -> PineRuntimeValue {
        guard length > 0 else { return .na }
        guard let prior = state.results.last?.number else { return sma(state.inputs, length) }
        guard let x = source.number else { return .na }
        let alpha = name == "ta.ema" ? 2.0 / Double(length + 1) : 1.0 / Double(length)
        return .float(alpha * x + (1 - alpha) * prior)
    }

    private static func extremum(
        _ inputs: [PineRuntimeValue], _ length: Int, pick: ([Double]) -> Double?
    ) -> PineRuntimeValue {
        lastValid(inputs, length).flatMap(pick).map(PineRuntimeValue.float) ?? .na
    }

    private static func change(_ inputs: [PineRuntimeValue], _ offset: Int) -> PineRuntimeValue {
        guard inputs.count > offset, let current = inputs[inputs.count - 1].number,
            let previous = inputs[inputs.count - 1 - offset].number
        else { return .na }
        return .float(current - previous)
    }

    private static func wma(_ inputs: [PineRuntimeValue], _ length: Int) -> PineRuntimeValue {
        guard let window = lastWindow(inputs, length) else { return .na }
        let weighted = window.enumerated().reduce(0.0) { $0 + Double($1.offset + 1) * $1.element }
        return .float(weighted / Double(length * (length + 1) / 2))
    }

    private static func stdev(_ inputs: [PineRuntimeValue], _ length: Int) -> PineRuntimeValue {
        guard let window = lastWindow(inputs, length) else { return .na }
        let mean = window.reduce(0, +) / Double(length)
        let variance = window.reduce(0) { $0 + ($1 - mean) * ($1 - mean) } / Double(length)
        return .float(sqrt(variance))
    }

    private static func trend(
        _ inputs: [PineRuntimeValue], _ length: Int, rising: Bool
    ) -> PineRuntimeValue {
        guard length > 0, let window = lastWindow(inputs, length + 1) else { return .bool(false) }
        return .bool(zip(window, window.dropFirst()).allSatisfy { rising ? $1 > $0 : $1 < $0 })
    }

    private static func momentum(
        _ inputs: [PineRuntimeValue], _ length: Int, isRate: Bool
    ) -> PineRuntimeValue {
        guard length > 0, inputs.count > length, let current = inputs[inputs.count - 1].number,
            let previous = inputs[inputs.count - 1 - length].number
        else { return .na }
        if !isRate { return .float(current - previous) }
        return previous == 0 ? .na : .float(100 * (current - previous) / previous)
    }

    private static func rsi(_ inputs: [PineRuntimeValue], _ length: Int) -> PineRuntimeValue {
        guard let values = lastValid(inputs, length + 1) else { return .na }
        let changes = zip(values.dropFirst(), values).map(-)
        let gain = changes.map { max($0, 0) }.reduce(0, +) / Double(length)
        let loss = changes.map { max(-$0, 0) }.reduce(0, +) / Double(length)
        return .float(loss == 0 ? 100 : 100 - (100 / (1 + gain / loss)))
    }

    private static func cross(
        _ name: String, _ source: PineRuntimeValue, _ second: PineRuntimeValue,
        prior: PineRuntimeValue?
    ) -> PineRuntimeValue {
        guard case .tuple(let previous)? = prior, previous.count == 2, let a = source.number,
            let b = second.number, let pa = previous[0].number, let pb = previous[1].number
        else { return .bool(false) }
        let crossedOver = a > b && pa <= pb
        let crossedUnder = a < b && pa >= pb
        switch name {
        case "ta.crossover": return .bool(crossedOver)
        case "ta.crossunder": return .bool(crossedUnder)
        default: return .bool(crossedOver || crossedUnder)
        }
    }

    // MARK: - MACD

    private static func exponentialAverage(_ values: [Double], _ length: Int) -> Double? {
        guard values.count >= length else { return nil }
        var e = values.prefix(length).reduce(0, +) / Double(length)
        for x in values.dropFirst(length) { e = (2 * x + Double(length - 1) * e) / Double(length + 1) }
        return e
    }

    private static func macd(_ state: State) -> PineRuntimeValue {
        let values = state.inputs.compactMap(\.number)
        let fast = exponentialAverage(values, macdFast)
        let slow = exponentialAverage(values, macdSlow)
        let line = fast.flatMap { f in slow.map { f - $0 } }
        let lineHistory =
            state.results.compactMap { result -> Double? in
                if case .tuple(let t) = result { return t.first?.number }
                return nil
            } + [line].compactMap { $0 }
        let signal = exponentialAverage(lineHistory, macdSignal)
        return .tuple([
            line.map(PineRuntimeValue.float) ?? .na, signal.map(PineRuntimeValue.float) ?? .na,
            line.flatMap { l in signal.map { .float(l - $0) } } ?? .na,
        ])
    }
}
