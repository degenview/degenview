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
    let theme: PineChartTheme
    private var committed = State(), working = State(), intrabar: [String: PineRuntimeValue] = [:]
    private var lastOpenTime: Date?
    /// `syminfo.mintick`. When not supplied it is inferred from bar prices on `evaluate`.
    private let suppliedMintick: Double?
    private var mintick: Double
    private let functions: [String: Function]
    private let symbol: PineSymbolInfo
    /// Seconds between bars, inferred on `evaluate` for `timeframe.*`.
    private var barSeconds: Double = 0
    private var isStrategy: Bool { program.declaration.type == .strategy }

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
        var fills: [Int: PineFillOutput] = [:]
        // Reference-typed objects live in the state so realtime rollback restores them
        // together with the variables that point at them.
        var arrays: [Int: [PineRuntimeValue]] = [:]
        var lines: [Int: PineLineOutput] = [:]
        var labels: [Int: PineLabelOutput] = [:]
        var boxes: [Int: PineBoxOutput] = [:]
        var tables: [Int: PineTableOutput] = [:]
        var candles: [Int: PineCandleOutput] = [:]
        var alerts: [PineAlertEvent] = []
        var broker = PineBrokerEmulator()
        var nextReference = 0
        var barIndex = -1
        var instructions = 0
    }

    private struct Function {
        let parameters: [PineParameter]
        let body: [PineStatement]
        /// Parameters and every name declared in the body; restored after each call.
        let locals: Set<String>
        /// `var`/`varip` locals, which persist per call site between calls.
        let persistentLocals: Set<String>
    }

    private enum Flow { case normal, breakLoop, continueLoop }

    private static let alertLimit = 200
    private static let defaultDrawingLimit = 50
    private static let defaultColor: UInt32 = 0x2196_f3ff

    init(
        program: PineCompiledProgram, inputs: [String: PineInputValue] = [:],
        limits: PineLimits = .default, mintick: Double? = nil, theme: PineChartTheme = .dark,
        symbol: PineSymbolInfo = PineSymbolInfo()
    ) {
        self.program = program
        self.symbol = symbol
        self.inputs = inputs
        self.limits = limits
        self.theme = theme
        self.suppliedMintick = mintick
        self.mintick = mintick ?? 0.01
        var functions: [String: Function] = [:]
        for statement in program.statements {
            guard case .function(let name, let parameters, let body, _) = statement else { continue }
            var locals = Set(parameters.map(\.name))
            var persistent = Set<String>()
            Self.collectDeclarations(body, into: &locals, persistent: &persistent)
            functions[name] = .init(
                parameters: parameters, body: body, locals: locals, persistentLocals: persistent)
        }
        self.functions = functions
        for input in program.inputSchema.inputs where self.inputs[input.id] == nil {
            self.inputs[input.id] = input.defaultValue
        }
        committed = freshState()
        working = committed
    }

    private func freshState() -> State {
        var state = State()
        state.broker = PineBrokerEmulator(settings: program.declaration.strategy ?? PineStrategySettings())
        return state
    }

    private func chartColor(_ name: String) -> UInt32? {
        switch name {
        case "chart.fg_color": theme.foreground
        case "chart.bg_color": theme.background
        default: nil
        }
    }

    private static func collectDeclarations(
        _ statements: [PineStatement], into names: inout Set<String>, persistent: inout Set<String>
    ) {
        for statement in statements {
            switch statement {
            case .declaration(let name, _, let mode, let expression, _):
                names.insert(name)
                if mode != .ordinary { persistent.insert(name) }
                if case .statementExpression(let block, _) = expression {
                    collectDeclarations([block], into: &names, persistent: &persistent)
                }
            case .conditional(_, let a, let b, _):
                collectDeclarations(a, into: &names, persistent: &persistent)
                collectDeclarations(b, into: &names, persistent: &persistent)
            case .forRange(let variable, _, _, _, let body, _):
                names.insert(variable)
                collectDeclarations(body, into: &names, persistent: &persistent)
            case .forIn(let index, let value, _, let body, _):
                if let index { names.insert(index) }
                names.insert(value)
                collectDeclarations(body, into: &names, persistent: &persistent)
            case .whileLoop(_, let body, _): collectDeclarations(body, into: &names, persistent: &persistent)
            case .switchStatement(_, let arms, _):
                for arm in arms { collectDeclarations(arm.body, into: &names, persistent: &persistent) }
            case .tupleDeclaration(let tupleNames, _, _): names.formUnion(tupleNames)
            default: break
            }
        }
    }

    func reset(inputs: [String: PineInputValue]? = nil) {
        if let inputs { self.inputs = inputs }
        committed = freshState()
        working = committed
        intrabar = [:]
        lastOpenTime = nil
    }

    func evaluate(bars: [KlineData]) throws -> PineRuntimeResult {
        reset(inputs: inputs)
        if suppliedMintick == nil { mintick = Self.inferredMintick(bars) }
        if bars.count > 1 {
            barSeconds = bars[bars.count - 1].openTime.timeIntervalSince(bars[bars.count - 2].openTime)
        }
        let start = Date()
        var states: [[String: Bool]] = []
        for (index, bar) in bars.enumerated() {
            if Task.isCancelled { throw PineDiagnostic.error("PINE8008", .cancellation, "Evaluation cancelled.", .zero) }
            if Date().timeIntervalSince(start) > limits.deadline {
                throw PineDiagnostic.error("PINE8007", .resource, "Evaluation deadline exceeded.", .zero)
            }
            states.append(
                try execute(.init(candle: bar, phase: .historical), isLast: index == bars.count - 1))
        }
        return .init(output: output(), diagnostics: [], barStates: states)
    }

    /// Smallest decimal step that represents every recent price exactly (capped at 1e-8).
    static func inferredMintick(_ bars: [KlineData]) -> Double {
        var decimals = 0
        for bar in bars.suffix(200) {
            for price in [bar.openPrice, bar.highPrice, bar.lowPrice, bar.closePrice] where price.isFinite {
                while decimals < 8 {
                    let scaled = price * pow(10, Double(decimals))
                    if abs(scaled - scaled.rounded()) <= max(1e-6, abs(scaled) * 1e-12) { break }
                    decimals += 1
                }
            }
        }
        return pow(10, -Double(decimals))
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
        if isStrategy {
            working.broker.process(bar: event.candle, barIndex: working.barIndex, mintick: mintick)
        }
        try run(program.statements, &context)
        if isStrategy {
            if program.declaration.strategy?.processOrdersOnClose == true {
                working.broker.fillMarketOrdersAtClose(
                    bar: event.candle, barIndex: working.barIndex, mintick: mintick)
            }
            working.broker.recordEquity(close: event.candle.closePrice)
        }
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
            overlay: program.declaration.overlay, barCount: working.barIndex + 1,
            plots: working.plots.values.sorted { $0.id < $1.id },
            hlines: working.hlines.values.sorted { $0.id < $1.id },
            markers: working.markers.values.sorted { $0.id < $1.id },
            backgrounds: working.backgrounds.values.sorted { $0.id < $1.id },
            barColors: working.barColors.values.sorted { $0.id < $1.id },
            fills: working.fills.values.sorted { $0.id < $1.id },
            lines: working.lines.values.sorted { $0.id < $1.id },
            labels: working.labels.values.sorted { $0.id < $1.id },
            boxes: working.boxes.values.sorted { $0.id < $1.id },
            tables: working.tables.values.sorted { $0.id < $1.id },
            candles: working.candles.values.sorted { $0.id < $1.id }, alerts: working.alerts,
            strategy: isStrategy ? working.broker.report() : nil)
    }
    private struct Context {
        var bar: KlineData
        var flags: [String: Bool]
        var modes: [String: PineDeclarationMode] = [:]
        /// Non-zero inside a user function: the caller's call-site key, so builtins
        /// called in the body keep separate histories per call site.
        var sitePrefix = 0
        var depth = 0
    }

    private func siteKey(_ site: Int, _ context: Context) -> Int {
        context.sitePrefix == 0 ? site : context.sitePrefix &* 1_000_003 &+ site
    }

    /// Runs a block. Returns how the block ended and the value of its last statement,
    /// which is a function's return value.
    @discardableResult
    private func run(_ statements: [PineStatement], _ context: inout Context) throws -> (
        Flow, PineRuntimeValue
    ) {
        var last = PineRuntimeValue.void
        for statement in statements {
            try budget()
            switch statement {
            case .declaration(let name, _, let mode, let expression, _):
                if mode == .variable, let existing = working.variables[name] {
                    last = existing
                    continue
                }
                if mode == .intrabar, let value = intrabar[name] ?? working.variables[name] {
                    working.variables[name] = value
                    intrabar[name] = value
                    context.modes[name] = mode
                    last = value
                    continue
                }
                let value = try eval(expression, &context)
                working.variables[name] = value
                context.modes[name] = mode
                if mode == .intrabar { intrabar[name] = value }
                last = value
            case .assignment(let name, let op, let expression, _):
                let rhs = try eval(expression, &context)
                let old = working.variables[name] ?? .na
                let value = PineOperators.apply(op, old: old, rhs)
                working.variables[name] = value
                if context.modes[name] == .intrabar || intrabar[name] != nil { intrabar[name] = value }
                last = value
            case .expression(let expression): last = try eval(expression, &context)
            case .conditional(let condition, let yes, let no, _):
                guard case .bool(let test) = try eval(condition, &context) else {
                    throw PineDiagnostic.error(
                        "PINE4001", .runtime,
                        "if condition must be bool; numeric-to-bool coercion is not allowed in v6.",
                        condition.range)
                }
                let (flow, value) = try run(test ? yes : no, &context)
                last = value
                if flow != .normal { return (flow, last) }
            case .forRange(let name, let from, let end, let step, let body, let range):
                let startValue = try eval(from, &context)
                guard let start = startValue.number, let stop = try eval(end, &context).number else {
                    continue
                }
                var stride = 1.0
                var integral: Bool = {
                    if case .int = startValue { return true }
                    return false
                }()
                if let step {
                    let stepValue = try eval(step, &context)
                    guard let size = stepValue.number, size != 0, size.isFinite else {
                        throw PineDiagnostic.error("PINE4014", .runtime, "for loop step must be a non-zero number.", range)
                    }
                    stride = abs(size)
                    if case .int = stepValue {} else { integral = false }
                }
                // Pine picks the direction from the bounds: `for i = 0 to -1` runs twice.
                let direction = stop >= start ? 1.0 : -1.0
                var i = start
                while direction > 0 ? i <= stop : i >= stop {
                    try budget()
                    working.variables[name] = integral ? .int(Int(i)) : .float(i)
                    let (flow, value) = try run(body, &context)
                    last = value
                    if flow == .breakLoop { break }
                    i += direction * stride
                }
            case .forIn(let indexName, let valueName, let collection, let body, let range):
                let target = try eval(collection, &context)
                if target == .na { continue }
                guard case .ref(.array, let id) = target else {
                    throw PineDiagnostic.error("PINE4011", .runtime, "for...in requires an array.", range)
                }
                // Iterate a snapshot: Pine forbids resizing the array inside the loop.
                for (offset, item) in (working.arrays[id] ?? []).enumerated() {
                    try budget()
                    if let indexName { working.variables[indexName] = .int(offset) }
                    working.variables[valueName] = item
                    let (flow, value) = try run(body, &context)
                    last = value
                    if flow == .breakLoop { break }
                }
            case .whileLoop(let condition, let body, _):
                while true {
                    try budget()
                    guard case .bool(let test) = try eval(condition, &context) else {
                        throw PineDiagnostic.error(
                            "PINE4001", .runtime, "while condition must be bool.", condition.range)
                    }
                    if !test { break }
                    let (flow, value) = try run(body, &context)
                    last = value
                    if flow == .breakLoop { break }
                }
            case .switchStatement(let subject, let arms, let range):
                let target = try subject.map { try eval($0, &context) }
                var chosen: [PineStatement]?
                for arm in arms {
                    guard let condition = arm.condition else {
                        chosen = chosen ?? arm.body
                        continue
                    }
                    let value = try eval(condition, &context)
                    if target != nil {
                        if PineOperators.apply(.equal, target!, value) == .bool(true) {
                            chosen = arm.body
                            break
                        }
                    } else {
                        guard case .bool(let test) = value else {
                            throw PineDiagnostic.error(
                                "PINE4001", .runtime, "switch arm condition must be bool.", range)
                        }
                        if test {
                            chosen = arm.body
                            break
                        }
                    }
                }
                if let chosen {
                    let (flow, value) = try run(chosen, &context)
                    last = value
                    if flow != .normal { return (flow, last) }
                } else {
                    last = .na
                }
            case .tupleDeclaration(let names, let expression, _):
                let value = try eval(expression, &context)
                if case .tuple(let items) = value {
                    for (i, name) in names.enumerated() where name != "_" {
                        working.variables[name] = i < items.count ? items[i] : .na
                    }
                } else {
                    for name in names where name != "_" { working.variables[name] = .na }
                }
                last = value
            case .loopControl(let control, _):
                return (control == .breakLoop ? .breakLoop : .continueLoop, last)
            case .function:
                continue
            }
        }
        return (.normal, last)
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
            if let color = PineBuiltins.colors[name] ?? chartColor(name) {
                return .color(color)
            }
            if let constant = PineBuiltins.constants[name] { return constant }
            return .string(name)
        case .unary(let op, let e, let range):
            let v = try eval(e, &context)
            switch op {
            case .negate: return PineOperators.negate(v)
            case .plus: return v
            case .not:
                guard case .bool(let b) = v else {
                    throw PineDiagnostic.error("PINE4002", .runtime, "not requires bool.", range)
                }
                return .bool(!b)
            }
        case .binary(let left, let op, let right, let range):
            let lhs = try eval(left, &context)
            if op == .and {
                guard case .bool(let b) = lhs else {
                    throw PineDiagnostic.error("PINE4003", .runtime, "and requires bool operands.", range)
                }
                if !b { return .bool(false) }
                guard case .bool(let r) = try eval(right, &context) else {
                    throw PineDiagnostic.error("PINE4003", .runtime, "and requires bool operands.", range)
                }
                return .bool(r)
            }
            if op == .or {
                guard case .bool(let b) = lhs else {
                    throw PineDiagnostic.error("PINE4004", .runtime, "or requires bool operands.", range)
                }
                if b { return .bool(true) }
                guard case .bool(let r) = try eval(right, &context) else {
                    throw PineDiagnostic.error("PINE4004", .runtime, "or requires bool operands.", range)
                }
                return .bool(r)
            }
            return PineOperators.apply(op, lhs, try eval(right, &context))
        case .ternary(let condition, let yes, let no, let range):
            guard case .bool(let b) = try eval(condition, &context) else {
                throw PineDiagnostic.error("PINE4005", .runtime, "Ternary condition must be bool.", range)
            }
            return try eval(b ? yes : no, &context)
        case .history(let base, let offset, let range):
            guard case .identifier(let name, _) = base, let n = try eval(offset, &context).number,
                n.isFinite
            else {
                throw PineDiagnostic.error(
                    "PINE4006", .runtime,
                    "History offset must be a non-negative integer and base must be a series.", range)
            }
            let i = Int(n)
            guard i >= 0 else {
                throw PineDiagnostic.error("PINE4006", .runtime, "History offset cannot be negative.", range)
            }
            if i == 0 { return try eval(base, &context) }
            let history = working.histories[name] ?? []
            return i <= history.count ? history[history.count - i] : .na
        case .tuple(let expressions, _): return .tuple(try expressions.map { try eval($0, &context) })
        case .statementExpression(let statement, _):
            let (_, value) = try run([statement], &context)
            return value == .void ? .na : value
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
        if let function = functions[name] {
            return try invoke(function, name, args, site, range, &context)
        }
        let key = siteKey(site, context)
        if name == "indicator" || name == "strategy" || name == "library" { return .void }
        if name.hasPrefix("input.") {
            guard case .identifier(let variable, _) = findDeclarationExpression(site: site) else {
                return try arg(0)
            }
            if let overridden = runtimeInput(inputs[variable], context: context) { return overridden }
            return try arg(0)
        }
        if name == "na" { return .bool(try arg(0) == .na) }
        if name == "nz" {
            let value = try arg(0)
            if value != .na { return value }
            let replacement = try arg(1)
            return replacement == .na ? .float(0) : replacement
        }
        switch name {
        case "int":
            guard let n = try arg(0).number, n.isFinite, abs(n) < 9e15 else { return .na }
            return .int(Int(n))
        case "float":
            return try arg(0).number.map(PineRuntimeValue.float) ?? .na
        case "bool":
            let v = try arg(0)
            if case .bool = v { return v }
            return v.number.map { .bool($0 != 0) } ?? .bool(false)
        case "line", "label", "box", "table":
            return try arg(0)
        default: break
        }
        if name.hasPrefix("math.") {
            return try math(name, args.indices.map { try arg($0) })
        }
        if name == "color.new" {
            guard case .color(let c) = try arg(0), let transparency = try arg(1).number else { return .na }
            return .color(PineBuiltins.withTransparency(c, transparency))
        }
        if name == "color.rgb" {
            return .color(
                PineBuiltins.rgb(
                    try arg(0).number ?? 0, try arg(1).number ?? 0, try arg(2).number ?? 0,
                    try arg(3).number ?? 0))
        }
        if name.hasPrefix("str.") && name != "str.tostring" {
            return try stringFunction(name, args.indices.map { try arg($0) }, range)
        }
        if name == "str.tostring" {
            let value = try arg(0)
            let pattern = textValue(try arg(1, "format"))
            return .string(format(value, pattern))
        }
        if name == "timestamp" {
            var positional: [PineRuntimeValue] = []
            var named: [String: PineRuntimeValue] = [:]
            for argument in args {
                let value = try eval(argument.value, &context)
                if let label = argument.name { named[label] = value } else { positional.append(value) }
            }
            return PineTimestamp.evaluate(positional: positional, named: named).map(PineRuntimeValue.int)
                ?? .na
        }
        if ["year", "month", "dayofmonth", "hour", "minute", "second", "dayofweek"].contains(name) {
            if args.isEmpty { return Self.timePart(name, Self.milliseconds(context.bar.openTime)) }
            guard let stamp = intValue(try arg(0)) else { return .na }
            return Self.timePart(name, stamp)
        }
        if name == "alert" || name == "alertcondition" {
            return try alertCall(name, args, key, &context)
        }
        if name.hasPrefix("strategy.") { return try strategyCall(name, args, range, &context) }
        if name.hasPrefix("ta.") {
            if name == "ta.atr" {
                // ATR is the RMA of true range; true range uses the previous close.
                let length = Int(try arg(0).number.flatMap { $0.isFinite ? $0 : nil } ?? 0)
                return ta("ta.rma", .float(trueRange(context.bar)), .na, length, key, context)
            }
            if name == "ta.tr" { return .float(trueRange(context.bar)) }
            if name == "ta.pivothigh" || name == "ta.pivotlow" {
                return try pivot(name, args, key, &context)
            }
            if name == "ta.barssince" {
                let condition = try arg(0)
                let prior = working.calls[key]?.last?.number
                let result: PineRuntimeValue
                if condition == .bool(true) {
                    result = .int(0)
                } else if let prior {
                    result = .int(Int(prior) + 1)
                } else {
                    result = .na
                }
                working.calls[key] = [result]
                return result
            }
            if name == "ta.cum" {
                let total = (working.calls[key]?.last?.number ?? 0) + (try arg(0).number ?? 0)
                working.calls[key] = [.float(total)]
                return .float(total)
            }
            if name == "ta.bb" {
                let source = try arg(0)
                let length = intValue(try arg(1)) ?? 0
                let multiplier = try arg(2).number ?? 2
                let basis = ta("ta.sma", source, .na, length, key, context)
                let deviation = ta("ta.stdev", source, .na, length, -(key &+ 1), context)
                guard let mid = basis.number, let spread = deviation.number else {
                    return .tuple([.na, .na, .na])
                }
                return .tuple([
                    .float(mid), .float(mid + multiplier * spread), .float(mid - multiplier * spread),
                ])
            }
            let source = try arg(0)
            let second = try arg(1)
            let length = Int(second.number.flatMap { $0.isFinite ? $0 : nil } ?? 0)
            return ta(name, source, second, length, key, context)
        }
        if name.hasPrefix("array.") { return try array(name, args, range, &context) }
        if ["line.", "label.", "box.", "table."].contains(where: name.hasPrefix) {
            return try drawing(name, args, range, &context)
        }
        if ["plot", "hline", "plotshape", "plotchar", "plotcandle", "bgcolor", "barcolor", "fill"]
            .contains(name)
        {
            return try visual(name, args, key, &context)
        }
        throw PineDiagnostic.error("PINE4007", .runtime, "Unknown or unsupported function '\(name)'.", range)
    }

    private func invoke(
        _ function: Function, _ name: String, _ args: [PineArgument], _ site: Int,
        _ range: PineSourceRange, _ context: inout Context
    ) throws -> PineRuntimeValue {
        guard context.depth < limits.callDepth else {
            throw PineDiagnostic.error("PINE8005", .resource, "Call depth limit exceeded in '\(name)'.", range)
        }
        // Arguments evaluate in the caller's scope, before any local is bound.
        let positional = args.filter { $0.name == nil }
        var bound: [String: PineRuntimeValue] = [:]
        for (i, parameter) in function.parameters.enumerated() {
            if let named = args.first(where: { $0.name == parameter.name }) {
                bound[parameter.name] = try eval(named.value, &context)
            } else if i < positional.count {
                bound[parameter.name] = try eval(positional[i].value, &context)
            } else if let fallback = parameter.defaultValue {
                bound[parameter.name] = try eval(fallback, &context)
            } else {
                bound[parameter.name] = .na
            }
        }
        let prefix = siteKey(site, context)
        func stashKey(_ local: String) -> String { "\u{0}fn:\(prefix):\(local)" }

        let saved = function.locals.map { ($0, working.variables[$0]) }
        for local in function.locals { working.variables[local] = nil }
        for local in function.persistentLocals { working.variables[local] = working.variables[stashKey(local)] }
        for (parameter, value) in bound { working.variables[parameter] = value }

        let callerPrefix = context.sitePrefix
        context.sitePrefix = prefix
        context.depth += 1
        defer {
            context.sitePrefix = callerPrefix
            context.depth -= 1
        }
        let (_, value) = try run(function.body, &context)

        for local in function.persistentLocals { working.variables[stashKey(local)] = working.variables[local] }
        for (local, previous) in saved { working.variables[local] = previous }
        return value
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

    private func math(_ name: String, _ values: [PineRuntimeValue]) throws -> PineRuntimeValue {
        let a = values.first ?? .na
        let numbers = values.map(\.number)
        switch name {
        case "math.max", "math.min":
            guard !numbers.isEmpty, !numbers.contains(where: { $0 == nil }) else { return .na }
            let all = numbers.compactMap { $0 }
            let result = name == "math.max" ? all.max()! : all.min()!
            let integral = values.allSatisfy {
                if case .int = $0 { return true }
                return false
            }
            return integral ? .int(Int(result)) : .float(result)
        case "math.abs":
            if case .int(let x) = a { return .int(abs(x)) }
            return a.number.map { .float(abs($0)) } ?? .na
        case "math.round":
            guard let x = a.number, x.isFinite else { return .na }
            if values.count > 1, let precision = numbers[1] {
                let scale = pow(10, precision.rounded())
                return .float((x * scale).rounded() / scale)
            }
            return abs(x) < 9e15 ? .int(Int(x.rounded())) : .float(x.rounded())
        case "math.floor", "math.ceil":
            guard let x = a.number, x.isFinite else { return .na }
            let r = name == "math.floor" ? x.rounded(.down) : x.rounded(.up)
            return abs(r) < 9e15 ? .int(Int(r)) : .float(r)
        case "math.sign":
            guard let x = a.number else { return .na }
            return .float(x > 0 ? 1 : x < 0 ? -1 : 0)
        case "math.sqrt": return a.number.map { .float(sqrt($0)) } ?? .na
        case "math.log": return a.number.map { .float(log($0)) } ?? .na
        case "math.exp": return a.number.map { .float(exp($0)) } ?? .na
        case "math.log10": return a.number.map { .float(log10($0)) } ?? .na
        case "math.sin": return a.number.map { .float(sin($0)) } ?? .na
        case "math.cos": return a.number.map { .float(cos($0)) } ?? .na
        case "math.tan": return a.number.map { .float(tan($0)) } ?? .na
        case "math.asin": return a.number.map { .float(asin($0)) } ?? .na
        case "math.acos": return a.number.map { .float(acos($0)) } ?? .na
        case "math.atan": return a.number.map { .float(atan($0)) } ?? .na
        case "math.todegrees": return a.number.map { .float($0 * 180 / .pi) } ?? .na
        case "math.toradians": return a.number.map { .float($0 * .pi / 180) } ?? .na
        case "math.avg":
            guard !numbers.isEmpty, !numbers.contains(where: { $0 == nil }) else { return .na }
            return .float(numbers.compactMap { $0 }.reduce(0, +) / Double(numbers.count))
        case "math.round_to_mintick":
            guard let x = a.number, x.isFinite else { return .na }
            return .float((x / mintick).rounded() * mintick)
        case "math.pow":
            guard let x = a.number, numbers.count > 1, let y = numbers[1] else { return .na }
            return .float(pow(x, y))
        default: return .na
        }
    }

    private func ta(
        _ name: String, _ source: PineRuntimeValue, _ second: PineRuntimeValue, _ length: Int,
        _ site: Int, _ context: Context
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
                    seed.count >= length && length > 0
                    ? .float(seed.suffix(length).reduce(0, +) / Double(length)) : .na
            } else if let x = source.number {
                result = .float(alpha * x + (1 - alpha) * prior!)
            } else {
                result = .na
            }
        case "ta.highest":
            let v = values.compactMap { $0 }
            result = v.count >= length && length > 0 ? .float(v.suffix(length).max()!) : .na
        case "ta.lowest":
            let v = values.compactMap { $0 }
            result = v.count >= length && length > 0 ? .float(v.suffix(length).min()!) : .na
        case "ta.change":
            let offset = length > 0 ? length : 1
            if values.count > offset, let current = values[values.count - 1],
                let previous = values[values.count - 1 - offset]
            {
                result = .float(current - previous)
            } else {
                result = .na
            }
        case "ta.wma":
            let window = values.suffix(length)
            if length > 0, window.count == length, !window.contains(where: { $0 == nil }) {
                let weighted = window.enumerated().reduce(0.0) { $0 + Double($1.offset + 1) * $1.element! }
                result = .float(weighted / Double(length * (length + 1) / 2))
            } else {
                result = .na
            }
        case "ta.stdev":
            let window = values.suffix(length)
            if length > 0, window.count == length, !window.contains(where: { $0 == nil }) {
                let numbers = window.map { $0! }
                let mean = numbers.reduce(0, +) / Double(length)
                result = .float(sqrt(numbers.reduce(0) { $0 + ($1 - mean) * ($1 - mean) } / Double(length)))
            } else {
                result = .na
            }
        case "ta.rising", "ta.falling":
            let window = values.suffix(length + 1)
            if length > 0, window.count == length + 1, !window.contains(where: { $0 == nil }) {
                let numbers = window.map { $0! }
                result = .bool(
                    zip(numbers, numbers.dropFirst()).allSatisfy {
                        name == "ta.rising" ? $1 > $0 : $1 < $0
                    })
            } else {
                result = .bool(false)
            }
        case "ta.mom", "ta.roc":
            if length > 0, values.count > length, let current = values[values.count - 1],
                let previous = values[values.count - 1 - length]
            {
                result =
                    name == "ta.mom"
                    ? .float(current - previous) : (previous == 0 ? .na : .float(100 * (current - previous) / previous))
            } else {
                result = .na
            }
        case "ta.cross", "ta.crossover", "ta.crossunder":
            // Registry history stores tuples for the two input series.
            let prior = history.last
            if case .tuple(let p)? = prior, p.count == 2, let a = source.number, let b = second.number,
                let pa = p[0].number, let pb = p[1].number
            {
                let crossedOver = a > b && pa <= pb
                let crossedUnder = a < b && pa >= pb
                switch name {
                case "ta.crossover": result = .bool(crossedOver)
                case "ta.crossunder": result = .bool(crossedUnder)
                default: result = .bool(crossedOver || crossedUnder)
                }
            } else {
                result = .bool(false)
            }
            working.calls[site] = (working.calls[site] ?? []) + [.tuple([source, second])]
            working.callInputs[site, default: []].append(source)
            return result
        case "ta.rsi":
            let raw = values.compactMap { $0 }
            guard raw.count > length else {
                result = .na
                break
            }
            let changes = zip(raw.dropFirst(), raw).map(-)
            let window = changes.suffix(length)
            let gain = window.map { max($0, 0) }.reduce(0, +) / Double(length)
            let loss = window.map { max(-$0, 0) }.reduce(0, +) / Double(length)
            result = .float(loss == 0 ? 100 : 100 - (100 / (1 + gain / loss)))
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
    private func emaFrom(_ values: [Double?], _ length: Int) -> Double? {
        let valid = values.compactMap { $0 }
        guard valid.count >= length else { return nil }
        var e = valid.prefix(length).reduce(0, +) / Double(length)
        for x in valid.dropFirst(length) { e = (2 * x + Double(length - 1) * e) / Double(length + 1) }
        return e
    }

    // MARK: - Strings

    private func stringFunction(_ name: String, _ values: [PineRuntimeValue], _ range: PineSourceRange)
        throws -> PineRuntimeValue
    {
        func text(_ index: Int) -> String? {
            if index < values.count, case .string(let value) = values[index] { return value }
            return nil
        }
        guard let subject = text(0) else {
            // `na` (or a non-string) in, `na` out — except the predicates, which are false.
            return ["str.contains", "str.startswith", "str.endswith"].contains(name) ? .bool(false) : .na
        }
        switch name {
        case "str.length": return .int(subject.count)
        case "str.upper": return .string(subject.uppercased())
        case "str.lower": return .string(subject.lowercased())
        case "str.trim": return .string(subject.trimmingCharacters(in: .whitespacesAndNewlines))
        case "str.contains": return .bool(text(1).map { $0.isEmpty || subject.contains($0) } ?? false)
        case "str.startswith": return .bool(text(1).map(subject.hasPrefix) ?? false)
        case "str.endswith": return .bool(text(1).map(subject.hasSuffix) ?? false)
        case "str.replace_all":
            guard let target = text(1), !target.isEmpty, let replacement = text(2) else { return .string(subject) }
            return .string(subject.replacingOccurrences(of: target, with: replacement))
        case "str.substring":
            let characters = Array(subject)
            guard values.count > 1, let begin = intValue(values[1]), begin >= 0, begin <= characters.count else {
                throw PineDiagnostic.error("PINE4016", .runtime, "str.substring begin index is out of range.", range)
            }
            let end = values.count > 2 ? (intValue(values[2]) ?? characters.count) : characters.count
            guard end >= begin, end <= characters.count else {
                throw PineDiagnostic.error("PINE4016", .runtime, "str.substring end index is out of range.", range)
            }
            return .string(String(characters[begin..<end]))
        case "str.tonumber":
            return Double(subject.trimmingCharacters(in: .whitespaces)).map(PineRuntimeValue.float) ?? .na
        case "str.format": return .string(formatTemplate(subject, Array(values.dropFirst())))
        default: throw PineDiagnostic.error("PINE4007", .runtime, "Unknown or unsupported function '\(name)'.", range)
        }
    }

    /// `str.format("{0} of {1,number,#.##}", a, b)`: `{n}` inserts the n-th argument,
    /// `{n,number,pattern}` formats it with a `str.tostring` pattern.
    private func formatTemplate(_ template: String, _ values: [PineRuntimeValue]) -> String {
        var output = ""
        var index = template.startIndex
        while index < template.endIndex {
            let character = template[index]
            guard character == "{", let close = template[index...].firstIndex(of: "}") else {
                output.append(character)
                index = template.index(after: index)
                continue
            }
            let parts = template[template.index(after: index)..<close].split(
                separator: ",", maxSplits: 2, omittingEmptySubsequences: false
            ).map { $0.trimmingCharacters(in: .whitespaces) }
            if let slot = parts.first.flatMap({ Int($0) }), values.indices.contains(slot) {
                output += format(values[slot], parts.count > 2 && parts[1] == "number" ? parts[2] : nil)
            } else {
                output += String(template[index...close])
            }
            index = template.index(after: close)
        }
        return output
    }

    private func trueRange(_ bar: KlineData) -> Double {
        let previous = working.histories["close"]?.last?.number
        return max(
            bar.highPrice - bar.lowPrice,
            max(
                previous.map { abs(bar.highPrice - $0) } ?? 0,
                previous.map { abs(bar.lowPrice - $0) } ?? 0))
    }

    /// `ta.pivothigh` / `ta.pivotlow`. The pivot is the bar `rightbars` back, so it is only
    /// known once that many later bars exist — the value appears on the confirming bar.
    /// The centre must be strictly beyond every left bar and at least as extreme as every
    /// right bar, so a flat top yields one pivot, at its first bar.
    private func pivot(
        _ name: String, _ args: [PineArgument], _ site: Int, _ context: inout Context
    ) throws -> PineRuntimeValue {
        let isHigh = name == "ta.pivothigh"
        let positional = args.filter { $0.name == nil }.count
        let b = try bind(
            args, positional == 2 ? ["leftbars", "rightbars"] : ["source", "leftbars", "rightbars"],
            &context)
        let source = b["source"] ?? market(isHigh ? "high" : "low", context) ?? .na
        guard let left = intValue(b["leftbars"]), let right = intValue(b["rightbars"]), left >= 0,
            right >= 0
        else { return .na }
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

    // MARK: - Alerts

    private func alertCall(
        _ name: String, _ args: [PineArgument], _ site: Int, _ context: inout Context
    ) throws -> PineRuntimeValue {
        if name == "alert" {
            let b = try bind(args, ["message", "freq"], &context)
            let message = b["message"].map { format($0, nil) } ?? ""
            recordAlert(
                message, freq: textValue(b["freq"]) ?? "alert.freq_once_per_bar", site: site, context)
        } else {
            let b = try bind(args, ["condition", "title", "message"], &context)
            guard b["condition"]?.bool == true else { return .void }
            recordAlert(
                textValue(b["message"]) ?? textValue(b["title"]) ?? "", freq: "alert.freq_all", site: site,
                context)
        }
        return .void
    }

    private func recordAlert(_ message: String, freq: String, site: Int, _ context: Context) {
        if freq == "alert.freq_once_per_bar_close", context.flags["barstate.isconfirmed"] != true { return }
        if freq == "alert.freq_once_per_bar",
            working.alerts.contains(where: { $0.site == site && $0.bar == working.barIndex })
        {
            return
        }
        working.alerts.append(
            .init(
                id: allocate(), site: site, bar: working.barIndex, time: context.bar.openTime,
                message: message))
        if working.alerts.count > Self.alertLimit {
            working.alerts.removeFirst(working.alerts.count - Self.alertLimit)
        }
    }

    // MARK: - Strategy

    private func strategyCall(
        _ name: String, _ args: [PineArgument], _ range: PineSourceRange, _ context: inout Context
    ) throws -> PineRuntimeValue {
        guard isStrategy else {
            throw PineDiagnostic.error("PINE4015", .runtime, "\(name) is only available in strategy() scripts.", range)
        }
        func positive(_ value: PineRuntimeValue?) -> Double? {
            guard let n = value?.number, n.isFinite, n > 0 else { return nil }
            return n
        }
        switch name {
        case "strategy.entry", "strategy.order":
            let b = try bind(
                args,
                [
                    "id", "direction", "qty", "limit", "stop", "oca_name", "oca_type", "comment",
                    "alert_message",
                ], &context)
            guard let id = textValue(b["id"]), let direction = textValue(b["direction"]) else {
                return .void
            }
            working.broker.place(
                .init(
                    kind: name == "strategy.entry" ? .entry : .order, id: id,
                    isLong: direction == "strategy.long", quantity: positive(b["qty"]),
                    limit: b["limit"]?.number, stop: b["stop"]?.number))
        case "strategy.exit":
            let b = try bind(
                args,
                [
                    "id", "from_entry", "qty", "qty_percent", "profit", "limit", "loss", "stop",
                    "trail_price", "trail_points", "trail_offset",
                ], &context)
            if b["trail_price"]?.number != nil || b["trail_points"]?.number != nil {
                throw PineDiagnostic.error(
                    "PINE9005", .unsupported, "strategy.exit trailing stops are not supported yet.", range)
            }
            guard let id = textValue(b["id"]) else { return .void }
            working.broker.place(
                .init(
                    kind: .exit, id: id, quantity: positive(b["qty"]),
                    quantityPercent: positive(b["qty_percent"]), limit: b["limit"]?.number,
                    stop: b["stop"]?.number, fromEntry: textValue(b["from_entry"]),
                    profitTicks: positive(b["profit"]), lossTicks: positive(b["loss"])))
        case "strategy.close":
            let b = try bind(args, ["id", "comment", "qty", "qty_percent"], &context)
            guard let id = textValue(b["id"]) else { return .void }
            working.broker.place(
                .init(
                    kind: .close, id: id, quantity: positive(b["qty"]),
                    quantityPercent: positive(b["qty_percent"])))
        case "strategy.close_all":
            working.broker.place(.init(kind: .closeAll, id: "Close position order"))
        case "strategy.cancel":
            if let id = textValue(try bind(args, ["id"], &context)["id"]) { working.broker.cancel(id: id) }
        case "strategy.cancel_all":
            working.broker.cancelAll()
        default:
            // `strategy.risk.*` limits are accepted and ignored.
            if !name.hasPrefix("strategy.risk.") {
                throw PineDiagnostic.error("PINE4007", .runtime, "Unknown or unsupported function '\(name)'.", range)
            }
        }
        return .void
    }

    private func strategyValue(_ name: String, _ bar: KlineData) -> PineRuntimeValue? {
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

    // MARK: - Symbol and time

    private static func milliseconds(_ date: Date) -> Int { Int(date.timeIntervalSince1970 * 1000) }

    private static func timePart(_ name: String, _ stamp: Int) -> PineRuntimeValue {
        let c = PineTimestamp.components(milliseconds: stamp)
        switch name {
        case "year": return .int(c.year)
        case "month": return .int(c.month)
        case "dayofmonth": return .int(c.day)
        case "hour": return .int(c.hour)
        case "minute": return .int(c.minute)
        case "second": return .int(c.second)
        default: return .int(c.weekday)
        }
    }

    /// `timeframe.*`, derived from the spacing of the bars the script runs over.
    private func timeframeValue(_ name: String) -> PineRuntimeValue? {
        let seconds = barSeconds
        guard seconds > 0 else { return nil }
        let day = 86_400.0
        let isMonthly = seconds >= 28 * day
        let isWeekly = !isMonthly && seconds >= 7 * day
        let isDaily = !isWeekly && !isMonthly && seconds >= day
        switch name {
        case "timeframe.period":
            if isMonthly { return .string("\(max(1, Int((seconds / (30 * day)).rounded())))M") }
            if isWeekly { return .string("\(max(1, Int((seconds / (7 * day)).rounded())))W") }
            if isDaily { return .string("\(max(1, Int((seconds / day).rounded())))D") }
            if seconds < 60 { return .string("\(Int(seconds))S") }
            return .string("\(Int((seconds / 60).rounded()))")
        case "timeframe.multiplier":
            if isMonthly { return .int(max(1, Int((seconds / (30 * day)).rounded()))) }
            if isWeekly { return .int(max(1, Int((seconds / (7 * day)).rounded()))) }
            if isDaily { return .int(max(1, Int((seconds / day).rounded()))) }
            return .int(seconds < 60 ? Int(seconds) : Int((seconds / 60).rounded()))
        case "timeframe.isdaily": return .bool(isDaily)
        case "timeframe.isweekly": return .bool(isWeekly)
        case "timeframe.ismonthly": return .bool(isMonthly)
        case "timeframe.isseconds": return .bool(seconds < 60)
        case "timeframe.isminutes": return .bool(seconds >= 60 && seconds < day)
        case "timeframe.isintraday": return .bool(seconds < day)
        case "timeframe.isdwm": return .bool(seconds >= day)
        default: return nil
        }
    }

    // MARK: - Arguments

    /// Binds Pine positional arguments to `names` in order; named arguments bind by name.
    private func bind(_ args: [PineArgument], _ names: [String], _ context: inout Context) throws
        -> [String: PineRuntimeValue]
    {
        var out: [String: PineRuntimeValue] = [:]
        var position = 0
        for argument in args {
            if let name = argument.name {
                out[name] = try eval(argument.value, &context)
            } else {
                if position < names.count { out[names[position]] = try eval(argument.value, &context) }
                position += 1
            }
        }
        return out
    }

    private func intValue(_ v: PineRuntimeValue?) -> Int? {
        guard let n = v?.number, n.isFinite, abs(n) < 9e15 else { return nil }
        return Int(n)
    }

    private func textValue(_ v: PineRuntimeValue?) -> String? {
        if case .string(let s)? = v { return s }
        return nil
    }

    /// Absent → `fallback`; a color → itself; `na` (or anything else) → no color.
    private func colorValue(_ v: PineRuntimeValue?, _ fallback: UInt32?) -> UInt32? {
        guard let v else { return fallback }
        if case .color(let c) = v { return c }
        return nil
    }

    // MARK: - Arrays

    private func allocate() -> Int {
        working.nextReference += 1
        return working.nextReference
    }

    private func array(
        _ name: String, _ args: [PineArgument], _ range: PineSourceRange, _ context: inout Context
    ) throws -> PineRuntimeValue {
        if name.hasPrefix("array.new_") {
            let b = try bind(args, ["size", "initial_value"], &context)
            let size = max(0, intValue(b["size"]) ?? 0)
            let id = allocate()
            working.arrays[id] = Array(repeating: b["initial_value"] ?? .na, count: size)
            return .ref(.array, id)
        }
        if name == "array.from" {
            let id = allocate()
            working.arrays[id] = try args.map { try eval($0.value, &context) }
            return .ref(.array, id)
        }
        let b = try bind(args, ["id", "index", "value"], &context)
        guard case .ref(.array, let id)? = b["id"], var items = working.arrays[id] else {
            throw PineDiagnostic.error("PINE4011", .runtime, "\(name) requires an array; got na.", range)
        }
        func index(_ key: String, allowEnd: Bool = false) throws -> Int {
            guard let i = intValue(b[key]), i >= 0, i < items.count + (allowEnd ? 1 : 0) else {
                throw PineDiagnostic.error(
                    "PINE4010", .runtime,
                    "\(name) index \(b[key]?.number.map { String(Int($0)) } ?? "na") is out of bounds (size \(items.count)).",
                    range)
            }
            return i
        }
        let numbers = items.compactMap(\.number)
        // `push`/`unshift`/`includes`/`indexof` take the value as their second argument.
        let element = b["value"] ?? b["index"] ?? .na
        var result: PineRuntimeValue = .void
        switch name {
        case "array.push": items.append(element)
        case "array.unshift": items.insert(element, at: 0)
        case "array.get": result = items[try index("index")]
        case "array.set": items[try index("index")] = b["value"] ?? .na
        case "array.insert": items.insert(b["value"] ?? .na, at: try index("index", allowEnd: true))
        case "array.remove": result = items.remove(at: try index("index"))
        case "array.size": result = .int(items.count)
        case "array.shift": result = items.isEmpty ? .na : items.removeFirst()
        case "array.pop": result = items.popLast() ?? .na
        case "array.first": result = items.first ?? .na
        case "array.last": result = items.last ?? .na
        case "array.clear": items.removeAll()
        case "array.includes": result = .bool(items.contains(element))
        case "array.indexof": result = .int(items.firstIndex(of: element) ?? -1)
        case "array.sum":
            let integral = items.allSatisfy {
                if case .int = $0 { return true }
                return $0 == .na
            }
            let total = numbers.reduce(0, +)
            result = integral ? .int(Int(total)) : .float(total)
        case "array.reverse": items.reverse()
        case "array.sort":
            let descending = textValue(b["order"] ?? b["index"]) == "order.descending"
            items.sort { lhs, rhs in
                switch (lhs.number, rhs.number) {
                case (let x?, let y?): return descending ? x > y : x < y
                case (nil, _?): return false
                case (_?, nil): return true
                default: return format(lhs, nil) < format(rhs, nil)
                }
            }
        case "array.copy":
            let copy = allocate()
            working.arrays[copy] = items
            result = .ref(.array, copy)
        case "array.concat":
            guard case .ref(.array, let otherID)? = b["index"], let other = working.arrays[otherID] else {
                throw PineDiagnostic.error("PINE4011", .runtime, "array.concat requires two arrays.", range)
            }
            items += other
            result = .ref(.array, id)
        case "array.slice":
            let from = try index("index", allowEnd: true)
            let to = intValue(b["value"]) ?? items.count
            guard to >= from, to <= items.count else {
                throw PineDiagnostic.error("PINE4010", .runtime, "array.slice end index \(to) is out of bounds.", range)
            }
            let slice = allocate()
            working.arrays[slice] = Array(items[from..<to])
            result = .ref(.array, slice)
        case "array.join":
            let separator = textValue(b["index"]) ?? ","
            result = .string(items.map { format($0, nil) }.joined(separator: separator))
        case "array.avg": result = numbers.isEmpty ? .na : .float(numbers.reduce(0, +) / Double(numbers.count))
        case "array.max": result = numbers.max().map(PineRuntimeValue.float) ?? .na
        case "array.min": result = numbers.min().map(PineRuntimeValue.float) ?? .na
        default:
            throw PineDiagnostic.error("PINE4007", .runtime, "Unknown or unsupported function '\(name)'.", range)
        }
        working.arrays[id] = items
        return result
    }

    // MARK: - Drawings

    private func prune<T>(_ objects: inout [Int: T], limit: Int?) {
        let limit = max(1, limit ?? Self.defaultDrawingLimit)
        while objects.count > limit, let oldest = objects.keys.min() { objects[oldest] = nil }
    }

    private func drawing(
        _ name: String, _ args: [PineArgument], _ range: PineSourceRange, _ context: inout Context
    ) throws -> PineRuntimeValue {
        let limits = program.declaration
        switch name {
        case "line.new":
            let b = try bind(
                args, ["x1", "y1", "x2", "y2", "xloc", "extend", "color", "style", "width"], &context)
            try requireBarIndex(b["xloc"], range)
            guard let x1 = intValue(b["x1"]), let y1 = b["y1"]?.number, let x2 = intValue(b["x2"]),
                let y2 = b["y2"]?.number
            else { return .na }
            let id = allocate()
            working.lines[id] = .init(
                id: id, x1: x1, y1: y1, x2: x2, y2: y2, color: colorValue(b["color"], Self.defaultColor) ?? 0,
                width: intValue(b["width"]) ?? 1, style: textValue(b["style"]) ?? "line.style_solid",
                extend: textValue(b["extend"]) ?? "extend.none")
            prune(&working.lines, limit: limits.maxLinesCount)
            return .ref(.line, id)
        case "label.new":
            let b = try bind(
                args,
                ["x", "y", "text", "xloc", "yloc", "color", "style", "textcolor", "size", "textalign"],
                &context)
            try requireBarIndex(b["xloc"], range)
            guard let x = intValue(b["x"]), let y = b["y"]?.number else { return .na }
            let id = allocate()
            working.labels[id] = .init(
                id: id, x: x, y: y, text: textValue(b["text"]) ?? "",
                color: colorValue(b["color"], Self.defaultColor),
                textColor: colorValue(b["textcolor"], 0x0000_00ff) ?? 0,
                style: textValue(b["style"]) ?? "label.style_label_down",
                size: textValue(b["size"]) ?? "size.normal")
            prune(&working.labels, limit: limits.maxLabelsCount)
            return .ref(.label, id)
        case "box.new":
            let b = try bind(
                args,
                [
                    "left", "top", "right", "bottom", "border_color", "border_width", "border_style",
                    "extend", "xloc", "bgcolor",
                ], &context)
            try requireBarIndex(b["xloc"], range)
            guard let left = intValue(b["left"]), let top = b["top"]?.number,
                let right = intValue(b["right"]), let bottom = b["bottom"]?.number
            else { return .na }
            let id = allocate()
            working.boxes[id] = .init(
                id: id, left: left, top: top, right: right, bottom: bottom,
                borderColor: colorValue(b["border_color"], Self.defaultColor),
                borderWidth: intValue(b["border_width"]) ?? 1,
                backgroundColor: colorValue(b["bgcolor"], Self.defaultColor))
            prune(&working.boxes, limit: limits.maxBoxesCount)
            return .ref(.box, id)
        case "table.new":
            let b = try bind(
                args,
                [
                    "position", "columns", "rows", "bgcolor", "frame_color", "frame_width",
                    "border_color", "border_width",
                ], &context)
            let id = allocate()
            working.tables[id] = .init(
                id: id, position: textValue(b["position"]) ?? "position.top_right",
                columns: max(0, intValue(b["columns"]) ?? 0), rows: max(0, intValue(b["rows"]) ?? 0),
                backgroundColor: colorValue(b["bgcolor"], nil), borderColor: colorValue(b["border_color"], nil),
                borderWidth: intValue(b["border_width"]) ?? 0, frameColor: colorValue(b["frame_color"], nil),
                frameWidth: intValue(b["frame_width"]) ?? 0)
            return .ref(.table, id)
        case "table.cell":
            let b = try bind(
                args,
                [
                    "table_id", "column", "row", "text", "width", "height", "text_color", "text_halign",
                    "text_valign", "text_size", "bgcolor",
                ], &context)
            guard case .ref(.table, let id)? = b["table_id"], var table = working.tables[id] else {
                return .void
            }
            guard let column = intValue(b["column"]), let row = intValue(b["row"]),
                (0..<table.columns).contains(column), (0..<table.rows).contains(row)
            else {
                throw PineDiagnostic.error(
                    "PINE4013", .runtime,
                    "table.cell position is outside the table's \(table.columns)×\(table.rows) grid.", range)
            }
            let cell = PineTableCell(
                column: column, row: row, text: textValue(b["text"]) ?? "",
                textColor: colorValue(b["text_color"], 0x0000_00ff) ?? 0,
                backgroundColor: colorValue(b["bgcolor"], nil),
                textSize: textValue(b["text_size"]) ?? "size.normal")
            table.cells.removeAll { $0.column == column && $0.row == row }
            table.cells.append(cell)
            working.tables[id] = table
            return .void
        default:
            return try mutateDrawing(name, args, range, &context)
        }
    }

    /// `line.set_*`, `label.set_*`, `box.set_*`, getters and `*.delete`. Operations on
    /// an `na` or deleted handle are no-ops (getters return `na`).
    private func mutateDrawing(
        _ name: String, _ args: [PineArgument], _ range: PineSourceRange, _ context: inout Context
    ) throws -> PineRuntimeValue {
        let values = try args.map { try eval($0.value, &context) }
        let target = values.first ?? .na
        let a = values.count > 1 ? values[1] : .na
        let b = values.count > 2 ? values[2] : .na
        let unsupported = PineDiagnostic.error("PINE4007", .runtime, "Unknown or unsupported function '\(name)'.", range)
        let member = String(name.split(separator: ".", maxSplits: 1).last ?? "")

        if name.hasPrefix("line.") {
            guard case .ref(.line, let id) = target, var line = working.lines[id] else {
                return member.hasPrefix("get_") ? .na : .void
            }
            switch member {
            case "delete":
                working.lines[id] = nil
                return .void
            case "set_x1": line.x1 = intValue(a) ?? line.x1
            case "set_x2": line.x2 = intValue(a) ?? line.x2
            case "set_y1": line.y1 = a.number ?? line.y1
            case "set_y2": line.y2 = a.number ?? line.y2
            case "set_xy1":
                line.x1 = intValue(a) ?? line.x1
                line.y1 = b.number ?? line.y1
            case "set_xy2":
                line.x2 = intValue(a) ?? line.x2
                line.y2 = b.number ?? line.y2
            case "set_color": line.color = colorValue(a, nil) ?? 0
            case "set_width": line.width = intValue(a) ?? line.width
            case "set_style": line.style = textValue(a) ?? line.style
            case "set_extend": line.extend = textValue(a) ?? line.extend
            case "get_x1": return .int(line.x1)
            case "get_x2": return .int(line.x2)
            case "get_y1": return .float(line.y1)
            case "get_y2": return .float(line.y2)
            default: throw unsupported
            }
            working.lines[id] = line
            return .void
        }
        if name.hasPrefix("label.") {
            guard case .ref(.label, let id) = target, var label = working.labels[id] else {
                return member.hasPrefix("get_") ? .na : .void
            }
            switch member {
            case "delete":
                working.labels[id] = nil
                return .void
            case "set_x": label.x = intValue(a) ?? label.x
            case "set_y": label.y = a.number ?? label.y
            case "set_xy":
                label.x = intValue(a) ?? label.x
                label.y = b.number ?? label.y
            case "set_text": label.text = textValue(a) ?? ""
            case "set_color": label.color = colorValue(a, nil)
            case "set_textcolor": label.textColor = colorValue(a, nil) ?? 0
            case "set_style": label.style = textValue(a) ?? label.style
            case "set_size": label.size = textValue(a) ?? label.size
            case "get_x": return .int(label.x)
            case "get_y": return .float(label.y)
            case "get_text": return .string(label.text)
            default: throw unsupported
            }
            working.labels[id] = label
            return .void
        }
        if name.hasPrefix("box.") {
            guard case .ref(.box, let id) = target, var box = working.boxes[id] else {
                return member.hasPrefix("get_") ? .na : .void
            }
            switch member {
            case "delete":
                working.boxes[id] = nil
                return .void
            case "set_left": box.left = intValue(a) ?? box.left
            case "set_right": box.right = intValue(a) ?? box.right
            case "set_top": box.top = a.number ?? box.top
            case "set_bottom": box.bottom = a.number ?? box.bottom
            case "set_lefttop":
                box.left = intValue(a) ?? box.left
                box.top = b.number ?? box.top
            case "set_rightbottom":
                box.right = intValue(a) ?? box.right
                box.bottom = b.number ?? box.bottom
            case "set_bgcolor": box.backgroundColor = colorValue(a, nil)
            case "set_border_color": box.borderColor = colorValue(a, nil)
            case "set_border_width": box.borderWidth = intValue(a) ?? box.borderWidth
            case "get_left": return .int(box.left)
            case "get_right": return .int(box.right)
            case "get_top": return .float(box.top)
            case "get_bottom": return .float(box.bottom)
            default: throw unsupported
            }
            working.boxes[id] = box
            return .void
        }
        if name == "table.delete" {
            if case .ref(.table, let id) = target { working.tables[id] = nil }
            return .void
        }
        throw unsupported
    }

    private func requireBarIndex(_ xloc: PineRuntimeValue?, _ range: PineSourceRange) throws {
        if textValue(xloc) == "xloc.bar_time" {
            throw PineDiagnostic.error("PINE9004", .unsupported, "xloc.bar_time drawings are not supported yet.", range)
        }
    }

    // MARK: - Visuals

    private func visual(_ name: String, _ args: [PineArgument], _ site: Int, _ context: inout Context)
        throws -> PineRuntimeValue
    {
        switch name {
        case "plot":
            let b = try bind(
                args, ["series", "title", "color", "linewidth", "style", "trackprice", "histbase"],
                &context)
            let color = colorValue(b["color"], Self.defaultColor) ?? 0
            var p =
                working.plots[site]
                ?? .init(
                    id: site, title: textValue(b["title"]), values: [], color: color,
                    lineWidth: intValue(b["linewidth"]) ?? 1, style: Self.plotStyle(textValue(b["style"])))
            p.values.append(b["series"]?.number)
            p.colors.append(color)
            p.display = intValue(b["display"]) ?? PineDisplay.all
            p.histBase = b["histbase"]?.number ?? 0
            working.plots[site] = p
            return .ref(.plot, site)
        case "hline":
            let b = try bind(args, ["price", "title", "color"], &context)
            if let n = b["price"]?.number {
                working.hlines[site] = .init(
                    id: site, value: n, color: colorValue(b["color"], Self.defaultColor) ?? 0,
                    title: textValue(b["title"]))
            }
            return .void
        case "plotshape", "plotchar":
            let isShape = name == "plotshape"
            let b = try bind(
                args, ["series", "title", isShape ? "style" : "char", "location", "color"], &context)
            let color = colorValue(b["color"], Self.defaultColor) ?? 0
            var m =
                working.markers[site]
                ?? .init(
                    id: site, kind: isShape ? .shape : .character, values: [],
                    character: isShape ? nil : textValue(b["char"]), color: color,
                    location: textValue(b["location"]) ?? "location.abovebar",
                    style: textValue(b["style"]) ?? "", size: textValue(b["size"]) ?? "size.auto")
            let markerValue = b["series"] ?? .na
            m.values.append(markerValue.bool ?? (markerValue.number != nil))
            m.prices.append(markerValue.number)
            m.colors.append(color)
            m.display = intValue(b["display"]) ?? PineDisplay.all
            working.markers[site] = m
            return .void
        case "fill":
            // `fill(p1, p2, top_value, bottom_value, top_color, bottom_color)` blends two colors
            // by price; `fill(p1, p2, color)` is flat.
            let isGradient =
                args.contains { $0.name == "top_value" } || args.filter { $0.name == nil }.count >= 5
            let b = try bind(
                args,
                isGradient
                    ? ["plot1", "plot2", "top_value", "bottom_value", "top_color", "bottom_color", "title"]
                    : ["plot1", "plot2", "color", "title"], &context)
            // Fills between hlines are not supported; only plot handles are.
            guard case .ref(.plot, let first)? = b["plot1"], case .ref(.plot, let second)? = b["plot2"] else {
                return .void
            }
            var f = working.fills[site] ?? .init(id: site, plotA: first, plotB: second, colors: [])
            if isGradient {
                f.colors.append(nil)
                if let top = b["top_value"]?.number, let bottom = b["bottom_value"]?.number,
                    let topColor = colorValue(b["top_color"], nil),
                    let bottomColor = colorValue(b["bottom_color"], nil)
                {
                    f.gradients.append(
                        .init(top: top, bottom: bottom, topColor: topColor, bottomColor: bottomColor))
                } else {
                    f.gradients.append(nil)
                }
            } else {
                f.colors.append(
                    colorValue(b["color"], PineBuiltins.withTransparency(Self.defaultColor, 90)))
            }
            working.fills[site] = f
            return .void
        case "plotcandle":
            let b = try bind(
                args,
                [
                    "open", "high", "low", "close", "title", "color", "wickcolor", "editable", "show_last",
                    "bordercolor",
                ], &context)
            var output =
                working.candles[site]
                ?? .init(id: site, title: textValue(b["title"]), bars: [])
            output.display = intValue(b["display"]) ?? PineDisplay.all
            if let open = b["open"]?.number, let high = b["high"]?.number, let low = b["low"]?.number,
                let close = b["close"]?.number
            {
                // An omitted color takes the up/down default; an explicit `na` hides that part.
                let fallback: UInt32 = close >= open ? 0x26a6_9aff : 0xef53_50ff
                let body = colorValue(b["color"], fallback)
                output.bars.append(
                    .init(
                        open: open, high: high, low: low, close: close, color: body,
                        wickColor: colorValue(b["wickcolor"], body),
                        borderColor: colorValue(b["bordercolor"], body)))
            } else {
                output.bars.append(nil)
            }
            working.candles[site] = output
            return .void
        default:
            let b = try bind(args, ["color"], &context)
            var c: PineColorOutput =
                (name == "bgcolor" ? working.backgrounds[site] : working.barColors[site])
                ?? .init(id: site, colors: [])
            c.colors.append(colorValue(b["color"] ?? .na, nil))
            if name == "bgcolor" { working.backgrounds[site] = c } else { working.barColors[site] = c }
            return .void
        }
    }

    private static func plotStyle(_ name: String?) -> PinePlotStyle {
        switch name {
        case "plot.style_stepline", "plot.style_steplinebr": .stepline
        case "plot.style_histogram": .histogram
        case "plot.style_columns": .columns
        case "plot.style_area", "plot.style_areabr": .area
        case "plot.style_circles": .circles
        case "plot.style_cross": .cross
        default: .line
        }
    }

    // MARK: - Values

    private func market(_ name: String, _ c: Context) -> PineRuntimeValue? {
        let bar = c.bar
        switch name {
        case "open": return .float(bar.openPrice)
        case "high": return .float(bar.highPrice)
        case "low": return .float(bar.lowPrice)
        case "close": return .float(bar.closePrice)
        case "volume": return .float(bar.volume)
        case "hl2": return .float((bar.highPrice + bar.lowPrice) / 2)
        case "hlc3": return .float((bar.highPrice + bar.lowPrice + bar.closePrice) / 3)
        case "ohlc4": return .float((bar.openPrice + bar.highPrice + bar.lowPrice + bar.closePrice) / 4)
        case "time": return .int(Int(bar.openTime.timeIntervalSince1970 * 1000))
        case "time_close": return .int(Int(bar.openTime.timeIntervalSince1970 * 1000))
        case "bar_index": return .int(working.barIndex)
        case "syminfo.mintick": return .float(mintick)
        case "syminfo.ticker": return .string(symbol.ticker)
        case "syminfo.tickerid": return .string(symbol.tickerID)
        case "syminfo.currency": return .string(symbol.currency)
        case "syminfo.type": return .string(symbol.type)
        case "ta.tr": return .float(trueRange(bar))
        case "year", "month", "dayofmonth", "hour", "minute", "second", "dayofweek":
            return Self.timePart(name, Self.milliseconds(bar.openTime))
        default:
            if name.hasPrefix("strategy.") { return strategyValue(name, bar) }
            if name.hasPrefix("timeframe.") { return timeframeValue(name) }
            return nil
        }
    }

    /// `str.tostring`. Patterns support `#`/`0` fraction digits (`"#.##"`, `"0.00"`) and
    /// `format.mintick`/`format.percent`.
    private func format(_ value: PineRuntimeValue, _ pattern: String?) -> String {
        switch value {
        case .string(let s): return s
        case .bool(let b): return b ? "true" : "false"
        case .int(let i) where pattern == nil: return String(i)
        case .int, .float: break
        default: return "NaN"
        }
        guard let x = value.number, x.isFinite else { return "NaN" }
        var minDigits = 0
        var maxDigits = 8
        var suffix = ""
        switch pattern {
        case nil, "format.volume", "format.inherit": break
        case "format.mintick":
            let decimals = max(0, Int((-log10(mintick)).rounded(.up)))
            (minDigits, maxDigits) = (decimals, decimals)
        case "format.percent":
            (minDigits, maxDigits, suffix) = (2, 2, "%")
        case let pattern?:
            if let dot = pattern.firstIndex(of: ".") {
                let fraction = pattern[pattern.index(after: dot)...]
                minDigits = fraction.filter { $0 == "0" }.count
                maxDigits = fraction.filter { $0 == "0" || $0 == "#" }.count
            } else {
                maxDigits = 0
            }
        }
        var text = String(format: "%.\(maxDigits)f", x)
        if maxDigits > minDigits, text.contains(".") {
            var trimmable = maxDigits - minDigits
            while trimmable > 0, text.hasSuffix("0") {
                text.removeLast()
                trimmable -= 1
            }
            if text.hasSuffix(".") { text.removeLast() }
        }
        if text == "-0" { text = "0" }
        return text + suffix
    }

    private func commitHistories(_ bar: KlineData) {
        var series: [String: PineRuntimeValue] = [
            "open": .float(bar.openPrice), "high": .float(bar.highPrice), "low": .float(bar.lowPrice),
            "close": .float(bar.closePrice), "volume": .float(bar.volume),
        ]
        if isStrategy {
            for name in [
                "strategy.position_size", "strategy.position_avg_price", "strategy.equity",
                "strategy.netprofit", "strategy.openprofit",
            ] {
                series[name] = strategyValue(name, bar)
            }
        }
        for (k, v) in series.merging(working.variables, uniquingKeysWith: { $1 }) {
            working.histories[k, default: []].append(v)
        }
    }
    private func runtimeInput(_ v: PineInputValue?, context: Context) -> PineRuntimeValue? {
        guard let v else { return nil }
        switch v {
        case .int(let x): return .int(x)
        case .float(let x): return .float(x)
        case .bool(let x): return .bool(x)
        case .string(let x): return .string(x)
        case .color(let x): return .color(x)
        case .source(let name): return market(name, context)
        }
    }
    private func budget() throws {
        working.instructions += 1
        if working.instructions > limits.instructionsPerBar {
            throw PineDiagnostic.error("PINE8004", .resource, "Per-bar instruction limit exceeded.", .zero)
        }
    }
}
