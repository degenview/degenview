import Foundation

struct PineRuntimeResult: Sendable {
    var output: PineVisualOutput
    var diagnostics: [PineDiagnostic]
    var barStates: [[String: Bool]]
}

final class PineRuntimeSession: @unchecked Sendable {
    let program: PineCompiledProgram
    private(set) var inputs: [String: PineInputValue]
    let limits: PineLimits
    private var committed = State(), working = State(), intrabar: [String: PineRuntimeValue] = [:]
    private var lastOpenTime: Date?

    private struct State {
        var variables: [String: PineRuntimeValue] = [:]
        var histories: [String: [PineRuntimeValue]] = [:]
        var calls: [Int: [PineRuntimeValue]] = [:]
        var callInputs: [Int: [PineRuntimeValue]] = [:]
        var plots: [Int: PinePlotOutput] = [:]
        var hlines: [Int: PineHorizontalLine] = [:]
        var markers: [Int: PineMarkerOutput] = [:]
        var backgrounds: [Int: PineColorOutput] = [:]
        var barColors: [Int: PineColorOutput] = [:]
        var barIndex = -1
        var instructions = 0
    }

    init(
        program: PineCompiledProgram, inputs: [String: PineInputValue] = [:],
        limits: PineLimits = .default
    ) {
        self.program = program
        self.inputs = inputs
        self.limits = limits
        for input in program.inputSchema.inputs where self.inputs[input.id] == nil {
            self.inputs[input.id] = input.defaultValue
        }
    }

    func reset(inputs: [String: PineInputValue]? = nil) {
        if let inputs { self.inputs = inputs }
        committed = State()
        working = State()
        intrabar = [:]
        lastOpenTime = nil
    }

    func evaluate(bars: [KlineData]) throws -> PineRuntimeResult {
        reset(inputs: inputs)
        let start = Date()
        var states: [[String: Bool]] = []
        for (index, bar) in bars.enumerated() {
            if Task.isCancelled { throw diag("PINE8008", .cancellation, "Evaluation cancelled.", .zero) }
            if Date().timeIntervalSince(start) > limits.deadline {
                throw diag("PINE8007", .resource, "Evaluation deadline exceeded.", .zero)
            }
            states.append(
                try execute(.init(candle: bar, phase: .historical), isLast: index == bars.count - 1))
        }
        return .init(output: output(), diagnostics: [], barStates: states)
    }

    @discardableResult func execute(_ event: PineBarEvent, isLast: Bool = true) throws -> [String:
        Bool]
    {
        let isNew = event.candle.openTime != lastOpenTime
        let confirmed: Bool
        let realtime: Bool
        switch event.phase {
        case .historical:
            confirmed = true
            realtime = false
        case .realtimeTick:
            confirmed = false
            realtime = true
        case .realtimeClose:
            confirmed = true
            realtime = true
        }
        working = committed
        working.barIndex = isNew ? committed.barIndex + 1 : max(0, committed.barIndex + 1)
        working.instructions = 0
        if !isNew { for (key, value) in intrabar { working.variables[key] = value } }
        let flags = [
            "barstate.isfirst": working.barIndex == 0, "barstate.islast": isLast,
            "barstate.ishistory": !realtime, "barstate.isrealtime": realtime, "barstate.isnew": isNew,
            "barstate.isconfirmed": confirmed, "barstate.islastconfirmedhistory": !realtime && isLast,
        ]
        var context = Context(bar: event.candle, flags: flags)
        try run(program.statements, &context)
        if confirmed {
            commitHistories(event.candle)
            committed = working
            intrabar = [:]
            lastOpenTime = event.candle.openTime
        } else {
            lastOpenTime = event.candle.openTime
        }
        return flags
    }

    func output() -> PineVisualOutput {
        .init(
            overlay: program.declaration.overlay, plots: working.plots.values.sorted { $0.id < $1.id },
            hlines: working.hlines.values.sorted { $0.id < $1.id },
            markers: working.markers.values.sorted { $0.id < $1.id },
            backgrounds: working.backgrounds.values.sorted { $0.id < $1.id },
            barColors: working.barColors.values.sorted { $0.id < $1.id })
    }
    private struct Context {
        var bar: KlineData
        var flags: [String: Bool]
        var modes: [String: PineDeclarationMode] = [:]
    }

    private func run(_ statements: [PineStatement], _ context: inout Context) throws {
        for statement in statements {
            try budget()
            switch statement {
            case .declaration(let name, _, let mode, let expression, _):
                if mode == .variable, working.variables[name] != nil {
                    continue
                }
                if mode == .intrabar, let value = intrabar[name] ?? working.variables[name] {
                    working.variables[name] = value
                    intrabar[name] = value
                    context.modes[name] = mode
                    continue
                }
                let value = try eval(expression, &context)
                working.variables[name] = value
                context.modes[name] = mode
                if mode == .intrabar { intrabar[name] = value }
            case .assignment(let name, let op, let expression, _):
                let rhs = try eval(expression, &context)
                let old = working.variables[name] ?? .na
                let value =
                    op == .reassign
                    ? rhs
                    : numeric(
                        old, rhs,
                        op == .plusAssign
                            ? .plus : op == .minusAssign ? .minus : op == .starAssign ? .star : .slash)
                working.variables[name] = value
                if context.modes[name] == .intrabar || intrabar[name] != nil { intrabar[name] = value }
            case .expression(let expression): _ = try eval(expression, &context)
            case .conditional(let condition, let yes, let no, _):
                guard case .bool(let test) = try eval(condition, &context) else {
                    throw diag(
                        "PINE4001", .runtime,
                        "if condition must be bool; numeric-to-bool coercion is not allowed in v6.",
                        condition.range)
                }
                try run(test ? yes : no, &context)
            }
        }
    }

    private func eval(_ expression: PineExpression, _ context: inout Context) throws
        -> PineRuntimeValue
    {
        try budget()
        switch expression {
        case .literal(let value, _): return value
        case .identifier(let name, _):
            if let value = working.variables[name] { return value }
            if let value = market(name, context) { return value }
            if let value = context.flags[name] { return .bool(value) }
            if let color = colors[name] { return .color(color) }
            return .string(name)
        case .unary(let op, let e, let range):
            let v = try eval(e, &context)
            switch op {
            case .minus: return v.number.map { .float(-$0) } ?? .na
            case .plus: return v
            case .not:
                guard case .bool(let b) = v else {
                    throw diag("PINE4002", .runtime, "not requires bool.", range)
                }
                return .bool(!b)
            default: return .na
            }
        case .binary(let left, let op, let right, let range):
            let lhs = try eval(left, &context)
            if op == .and {
                guard case .bool(let b) = lhs else {
                    throw diag("PINE4003", .runtime, "and requires bool operands.", range)
                }
                if !b { return .bool(false) }
                guard case .bool(let r) = try eval(right, &context) else {
                    throw diag("PINE4003", .runtime, "and requires bool operands.", range)
                }
                return .bool(r)
            }
            if op == .or {
                guard case .bool(let b) = lhs else {
                    throw diag("PINE4004", .runtime, "or requires bool operands.", range)
                }
                if b { return .bool(true) }
                guard case .bool(let r) = try eval(right, &context) else {
                    throw diag("PINE4004", .runtime, "or requires bool operands.", range)
                }
                return .bool(r)
            }
            let rhs = try eval(right, &context)
            if [.equal, .notEqual, .less, .lessEqual, .greater, .greaterEqual].contains(op) {
                return compare(lhs, rhs, op)
            }
            return numeric(lhs, rhs, op)
        case .ternary(let condition, let yes, let no, let range):
            guard case .bool(let b) = try eval(condition, &context) else {
                throw diag("PINE4005", .runtime, "Ternary condition must be bool.", range)
            }
            return try eval(b ? yes : no, &context)
        case .history(let base, let offset, let range):
            guard case .identifier(let name, _) = base, let n = try eval(offset, &context).number else {
                throw diag(
                    "PINE4006", .runtime,
                    "History offset must be a non-negative integer and base must be a series.", range)
            }
            let i = Int(n)
            guard i >= 0 else {
                throw diag("PINE4006", .runtime, "History offset cannot be negative.", range)
            }
            if i == 0 { return try eval(base, &context) }
            let history = working.histories[name] ?? []
            return i <= history.count ? history[history.count - i] : .na
        case .tuple(let expressions, _): return .tuple(try expressions.map { try eval($0, &context) })
        case .call(let name, let args, let site, let range):
            return try call(name, args, site, range, &context)
        }
    }

    private func call(
        _ name: String, _ args: [PineArgument], _ site: Int, _ range: PineSourceRange,
        _ context: inout Context
    ) throws -> PineRuntimeValue {
        func arg(_ index: Int, _ key: String? = nil) throws -> PineRuntimeValue {
            if let key, let found = args.first(where: { $0.name == key }) {
                return try eval(found.value, &context)
            }
            guard index < args.count else { return .na }
            return try eval(args[index].value, &context)
        }
        if name == "indicator" { return .void }
        if name.hasPrefix("input.") {
            guard case .identifier(let variable, _) = findDeclarationExpression(site: site) else {
                return try arg(0)
            }
            if let overridden = runtimeInput(inputs[variable]) { return overridden }
            return try arg(0)
        }
        if name == "na" { return .bool(try arg(0) == .na) }
        if name == "nz" {
            let value = try arg(0)
            if value != .na { return value }
            let replacement = try arg(1)
            return replacement == .na ? .float(0) : replacement
        }
        if name.hasPrefix("math.") {
            let a = try arg(0)
            let b = try arg(1)
            switch name {
            case "math.max": return .float(max(a.number ?? .nan, b.number ?? .nan))
            case "math.min": return .float(min(a.number ?? .nan, b.number ?? .nan))
            case "math.abs": return .float(abs(a.number ?? .nan))
            case "math.sqrt": return .float(sqrt(a.number ?? .nan))
            case "math.pow": return .float(pow(a.number ?? .nan, b.number ?? .nan))
            default: return .na
            }
        }
        if name == "color.new" {
            guard case .color(let c) = try arg(0), let alpha = try arg(1).number else { return .na }
            return .color((c & 0xffff_ff00) | UInt32((100 - min(100, max(0, alpha))) * 2.55))
        }
        if name == "color.rgb" {
            let r = UInt32(try arg(0).number ?? 0)
            let g = UInt32(try arg(1).number ?? 0)
            let b = UInt32(try arg(2).number ?? 0)
            return .color((r << 24) | (g << 16) | (b << 8) | 255)
        }
        if name.hasPrefix("ta.") {
            let source = try arg(0)
            let length = Int(try arg(1).number ?? 0)
            return ta(name, source, length, site, context)
        }
        if ["plot", "hline", "plotshape", "plotchar", "bgcolor", "barcolor"].contains(name) {
            return try visual(name, args, site, &context)
        }
        throw diag("PINE4007", .runtime, "Unknown or unsupported function '\(name)'.", range)
    }

    // Input calls are declaration initializers. Finding the owning variable keeps IDs stable
    // even when titles change, and input overrides therefore need no recompilation.
    private func findDeclarationExpression(site: Int) -> PineExpression? {
        for s in program.statements {
            if case .declaration(let n, _, _, let e, _) = s, case .call(_, _, let id, _) = e, id == site {
                return .identifier(n, e.range)
            }
        }
        return nil
    }

    private func ta(
        _ name: String, _ source: PineRuntimeValue, _ length: Int, _ site: Int, _ context: Context
    ) -> PineRuntimeValue {
        var history = working.calls[site] ?? []
        var inputHistory = working.callInputs[site] ?? []
        let values = (inputHistory + [source]).map { $0.number }
        let result: PineRuntimeValue
        switch name {
        case "ta.sma":
            let valid = values.compactMap { $0 }
            result =
                valid.count >= length && length > 0
                ? .float(valid.suffix(length).reduce(0, +) / Double(length)) : .na
        case "ta.ema", "ta.rma":
            let alpha = name == "ta.ema" ? 2.0 / Double(length + 1) : 1.0 / Double(length)
            let prior = history.last?.number
            if prior == nil {
                let seed = values.compactMap { $0 }
                result =
                    seed.count >= length ? .float(seed.suffix(length).reduce(0, +) / Double(length)) : .na
            } else if let x = source.number {
                result = .float(alpha * x + (1 - alpha) * prior!)
            } else {
                result = .na
            }
        case "ta.highest":
            let v = values.compactMap { $0 }
            result = v.count >= length ? .float(v.suffix(length).max()!) : .na
        case "ta.lowest":
            let v = values.compactMap { $0 }
            result = v.count >= length ? .float(v.suffix(length).min()!) : .na
        case "ta.crossover", "ta.crossunder":
            // Registry history stores tuples for the two input series.
            let other = (try? argumentValueForTA(site: site, index: 1, context: context)) ?? .na
            let prior = history.last
            if case .tuple(let p)? = prior, p.count == 2, let a = source.number, let b = other.number,
                let pa = p[0].number, let pb = p[1].number
            {
                result = .bool(name == "ta.crossover" ? a > b && pa <= pb : a < b && pa >= pb)
            } else {
                result = .bool(false)
            }
            working.calls[site] = (working.calls[site] ?? []) + [.tuple([source, other])]
            working.callInputs[site, default: []].append(source)
            return result
        case "ta.rsi":
            let raw = marketHistoryClose() + [context.bar.closePrice]
            guard raw.count > length else {
                result = .na
                break
            }
            let changes = zip(raw.dropFirst(), raw).map(-)
            let window = changes.suffix(length)
            let gain = window.map { max($0, 0) }.reduce(0, +) / Double(length)
            let loss = window.map { max(-$0, 0) }.reduce(0, +) / Double(length)
            result = .float(loss == 0 ? 100 : 100 - (100 / (1 + gain / loss)))
        case "ta.atr":
            let previous = working.histories["close"]?.last?.number
            let tr = max(
                context.bar.highPrice - context.bar.lowPrice,
                max(
                    previous.map { abs(context.bar.highPrice - $0) } ?? 0,
                    previous.map { abs(context.bar.lowPrice - $0) } ?? 0))
            let prior = history.last?.number
            result =
                prior.map { .float(($0 * Double(length - 1) + tr) / Double(length)) }
                ?? (history.count + 1 >= length
                    ? .float(((history.compactMap { $0.number }.reduce(0, +)) + tr) / Double(length)) : .na)
        case "ta.macd":
            let fast = emaFrom(values, 12)
            let slow = emaFrom(values, 26)
            let macd = (fast != nil && slow != nil) ? fast! - slow! : nil
            let macdHistory =
                history.compactMap {
                    if case .tuple(let t) = $0 { return t.first?.number }
                    return nil
                } + [macd].compactMap { $0 }
            let signal = emaFrom(macdHistory.map(Optional.some), 9)
            result = .tuple([
                macd.map(PineRuntimeValue.float) ?? .na, signal.map(PineRuntimeValue.float) ?? .na,
                (macd != nil && signal != nil) ? .float(macd! - signal!) : .na,
            ])
        default: result = .na
        }
        history.append(result)
        inputHistory.append(source)
        working.calls[site] = history
        working.callInputs[site] = inputHistory
        return result
    }
    private func argumentValueForTA(site: Int, index: Int, context: Context) throws
        -> PineRuntimeValue
    {
        func search(_ ss: [PineStatement]) -> PineExpression? {
            for s in ss {
                switch s {
                case .declaration(_, _, _, let e, _), .expression(let e):
                    if case .call(_, let a, let id, _) = e, id == site, index < a.count {
                        return a[index].value
                    }
                case .conditional(_, let a, let b, _): if let e = search(a) ?? search(b) { return e }
                default: break
                }
            }
            return nil
        }
        guard let e = search(program.statements) else { return .na }
        var c = context
        return try eval(e, &c)
    }
    private func emaFrom(_ values: [Double?], _ length: Int) -> Double? {
        let valid = values.compactMap { $0 }
        guard valid.count >= length else { return nil }
        var e = valid.prefix(length).reduce(0, +) / Double(length)
        for x in valid.dropFirst(length) { e = (2 * x + Double(length - 1) * e) / Double(length + 1) }
        return e
    }

    private func visual(_ name: String, _ args: [PineArgument], _ site: Int, _ context: inout Context)
        throws -> PineRuntimeValue
    {
        func value(_ i: Int) throws -> PineRuntimeValue {
            i < args.count ? try eval(args[i].value, &context) : .na
        }
        func named(_ key: String) throws -> PineRuntimeValue {
            if let a = args.first(where: { $0.name == key }) { return try eval(a.value, &context) }
            return .na
        }
        let color: UInt32 = {
            if case .color(let c) = (try? named("color")) { return c }
            return 0x2196_f3ff
        }()
        if name == "plot" {
            var p =
                working.plots[site]
                ?? .init(
                    id: site, title: nil, values: [], color: color,
                    lineWidth: Int((try? named("linewidth").number) ?? 1), style: .line)
            p.values.append(try value(0).number)
            working.plots[site] = p
            return .void
        }
        if name == "hline", let n = try value(0).number {
            working.hlines[site] = .init(id: site, value: n, color: color, title: nil)
            return .void
        }
        if name == "plotshape" || name == "plotchar" {
            var m =
                working.markers[site]
                ?? .init(
                    id: site, kind: name == "plotshape" ? .shape : .character, values: [],
                    character: name == "plotchar"
                        ? ((try? value(2)).flatMap {
                            if case .string(let s) = $0 { return s }
                            return nil
                        }) : nil, color: color, location: "abovebar", style: "circle")
            m.values.append((try value(0).bool) ?? false)
            working.markers[site] = m
            return .void
        }
        var c: PineColorOutput = working.backgrounds[site] ?? .init(id: site, colors: [])
        let v = try value(0)
        c.colors.append(
            {
                if case .color(let x) = v { return x }
                return nil
            }())
        if name == "bgcolor" { working.backgrounds[site] = c } else { working.barColors[site] = c }
        return .void
    }

    private func market(_ name: String, _ c: Context) -> PineRuntimeValue? {
        switch name {
        case "open": return .float(c.bar.openPrice)
        case "high": return .float(c.bar.highPrice)
        case "low": return .float(c.bar.lowPrice)
        case "close": return .float(c.bar.closePrice)
        case "volume": return .float(c.bar.volume)
        case "time": return .int(Int(c.bar.openTime.timeIntervalSince1970 * 1000))
        case "time_close": return .int(Int(c.bar.openTime.timeIntervalSince1970 * 1000))
        case "bar_index": return .int(working.barIndex)
        default: return nil
        }
    }
    private let colors: [String: UInt32] = [
        "color.aqua": 0x00bc_d4ff, "color.black": 0x0000_00ff, "color.blue": 0x2196_f3ff,
        "color.fuchsia": 0xe040_fbff, "color.gray": 0x787b_86ff, "color.green": 0x4caf_50ff,
        "color.lime": 0x00e6_76ff, "color.maroon": 0x880e_4fff, "color.navy": 0x0d47_a1ff,
        "color.olive": 0x8277_17ff, "color.orange": 0xff98_00ff, "color.purple": 0x9c27_b0ff,
        "color.red": 0xf236_45ff, "color.silver": 0xb2b5_beff, "color.teal": 0x0089_7bff,
        "color.white": 0xffff_ffff, "color.yellow": 0xffeb_3bff,
    ]
    private func numeric(_ a: PineRuntimeValue, _ b: PineRuntimeValue, _ op: PineTokenKind)
        -> PineRuntimeValue
    {
        guard let x = a.number, let y = b.number else { return .na }
        switch op {
        case .plus: return .float(x + y)
        case .minus: return .float(x - y)
        case .star: return .float(x * y)
        case .slash: return y == 0 ? .na : .float(x / y)
        case .percent: return y == 0 ? .na : .float(x.truncatingRemainder(dividingBy: y))
        case .power: return .float(pow(x, y))
        default: return .na
        }
    }
    private func compare(_ a: PineRuntimeValue, _ b: PineRuntimeValue, _ op: PineTokenKind)
        -> PineRuntimeValue
    {
        if a == .na || b == .na { return .bool(false) }
        if let x = a.number, let y = b.number {
            switch op {
            case .equal: return .bool(x == y)
            case .notEqual: return .bool(x != y)
            case .less: return .bool(x < y)
            case .lessEqual: return .bool(x <= y)
            case .greater: return .bool(x > y)
            case .greaterEqual: return .bool(x >= y)
            default: break
            }
        }
        return .bool(op == .equal ? a == b : a != b)
    }
    private func commitHistories(_ bar: KlineData) {
        let market: [String: PineRuntimeValue] = [
            "open": .float(bar.openPrice), "high": .float(bar.highPrice), "low": .float(bar.lowPrice),
            "close": .float(bar.closePrice), "volume": .float(bar.volume),
        ]
        for (k, v) in market.merging(working.variables, uniquingKeysWith: { $1 }) {
            working.histories[k, default: []].append(v)
        }
    }
    private func marketHistoryClose() -> [Double] {
        working.histories["close", default: []].compactMap { $0.number }
    }
    private func runtimeInput(_ v: PineInputValue?) -> PineRuntimeValue? {
        guard let v else { return nil }
        switch v {
        case .int(let x): return .int(x)
        case .float(let x): return .float(x)
        case .bool(let x): return .bool(x)
        case .string(let x): return .string(x)
        }
    }
    private func budget() throws {
        working.instructions += 1
        if working.instructions > limits.instructionsPerBar {
            throw diag("PINE8004", .resource, "Per-bar instruction limit exceeded.", .zero)
        }
    }
}
