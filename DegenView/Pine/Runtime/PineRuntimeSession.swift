import Foundation

/// Interprets a compiled Pine program bar by bar. The behaviour is spread over extensions:
/// statements, expressions and the call router, plus one file per builtin family.
///
/// A session is confined to the thread that created it: `ChartViewModel` builds one inside its
/// detached evaluation task and never shares it.
final class PineRuntimeSession {
    let program: PineCompiledProgram
    private(set) var inputs: [String: PineInputValue]
    let limits: PineLimits
    let theme: PineChartTheme
    let symbol: PineSymbolInfo
    let functions: [String: PineRuntimeFunction]
    /// Variable each top-level `input.*` call site initialises, so input overrides stay keyed
    /// by name even when titles change.
    let inputVariables: [Int: String]
    var committed = PineRuntimeState()
    var working = PineRuntimeState()
    /// `varip` values, which survive the rollback between realtime ticks of one bar.
    var intrabar: [String: PineRuntimeValue] = [:]
    var lastOpenTime: Date?
    /// `syminfo.mintick`. When not supplied it is inferred from bar prices on `evaluate`.
    private let suppliedMintick: Double?
    var mintick: Double
    /// Seconds between bars, learned from consecutive bars, for `timeframe.*` and `time_close`.
    var barSeconds: Double = 0
    var isStrategy: Bool { program.declaration.type == .strategy }

    static let alertLimit = 200
    static let defaultDrawingLimit = 50
    static let defaultColor: UInt32 = 0x2196_f3ff
    private static let defaultMintick = 0.01

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
        let usableMintick = mintick.flatMap { $0 > 0 && $0.isFinite ? $0 : nil }
        self.suppliedMintick = usableMintick
        self.mintick = usableMintick ?? Self.defaultMintick
        var functions: [String: PineRuntimeFunction] = [:]
        var inputVariables: [Int: String] = [:]
        for statement in program.statements {
            switch statement {
            case .function(let name, let parameters, let body, _):
                functions[name] = .init(parameters: parameters, body: body)
            case .declaration(let name, _, _, .call(_, _, let site, _), _):
                inputVariables[site] = name
            default: break
            }
        }
        self.functions = functions
        self.inputVariables = inputVariables
        for input in program.inputSchema.inputs where self.inputs[input.id] == nil {
            self.inputs[input.id] = input.defaultValue
        }
        committed = freshState()
        working = committed
    }

    private func freshState() -> PineRuntimeState {
        var state = PineRuntimeState()
        state.broker = PineBrokerEmulator(settings: program.declaration.strategy ?? PineStrategySettings())
        return state
    }

    func reset(inputs: [String: PineInputValue]? = nil) {
        if let inputs { self.inputs = inputs }
        committed = freshState()
        working = committed
        intrabar = [:]
        lastOpenTime = nil
        barSeconds = 0
        mintick = suppliedMintick ?? Self.defaultMintick
    }

    // MARK: - Running

    func evaluate(bars: [KlineData]) throws -> PineRuntimeResult {
        reset(inputs: inputs)
        if suppliedMintick == nil { mintick = Self.inferredMintick(bars) }
        if bars.count > 1 {
            barSeconds = bars[bars.count - 1].openTime.timeIntervalSince(bars[bars.count - 2].openTime)
        }
        let start = Date()
        var states: [PineBarFlags] = []
        states.reserveCapacity(bars.count)
        for (index, bar) in bars.enumerated() {
            if Task.isCancelled {
                throw PineDiagnostic.error("PINE8008", .cancellation, "Evaluation cancelled.", .zero)
            }
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

    /// Runs the script on one bar event. Historical bars and confirmed realtime bars commit;
    /// a realtime tick runs on a scratch copy of the state.
    @discardableResult
    func execute(_ event: PineBarEvent, isLast: Bool = true) throws -> PineBarFlags {
        let isNew = event.candle.openTime != lastOpenTime
        if isNew, let previous = lastOpenTime {
            barSeconds = event.candle.openTime.timeIntervalSince(previous)
        }
        let confirmed = event.phase.isConfirmed
        let realtime = event.phase.isRealtime
        working = committed
        working.barIndex = committed.barIndex + 1
        working.instructions = 0
        if !isNew { for (key, value) in intrabar { working.variables[key] = value } }
        let flags = PineBarFlags(
            isFirst: working.barIndex == 0, isLast: isLast, isHistory: !realtime, isRealtime: realtime,
            isNew: isNew, isConfirmed: confirmed, isLastConfirmedHistory: !realtime && isLast)
        var context = PineRuntimeContext(bar: event.candle, flags: flags)
        if isStrategy {
            working.broker.process(bar: event.candle, barIndex: working.barIndex, mintick: mintick)
        }
        try run(program.statements, &context)
        if isStrategy { finishStrategyBar(event.candle) }
        if confirmed {
            commitHistories(event.candle)
            committed = working
            intrabar = [:]
        }
        lastOpenTime = event.candle.openTime
        return flags
    }

    private func finishStrategyBar(_ candle: KlineData) {
        if program.declaration.strategy?.processOrdersOnClose == true {
            working.broker.fillMarketOrdersAtClose(
                bar: candle, barIndex: working.barIndex, mintick: mintick)
        }
        working.broker.recordEquity(close: candle.closePrice)
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

    // MARK: - Bookkeeping

    /// Names the interpreter stashes per-call-site locals under; never scripts' own variables.
    static let internalPrefix = "\u{0}"

    private static let strategySeries = [
        "strategy.position_size", "strategy.position_avg_price", "strategy.equity",
        "strategy.netprofit", "strategy.openprofit",
    ]

    /// Appends the confirmed bar's value of every series and variable to its history, which is
    /// what `x[n]` reads.
    private func commitHistories(_ bar: KlineData) {
        var series: [String: PineRuntimeValue] = [
            "open": .float(bar.openPrice), "high": .float(bar.highPrice), "low": .float(bar.lowPrice),
            "close": .float(bar.closePrice), "volume": .float(bar.volume),
        ]
        if isStrategy {
            for name in Self.strategySeries { series[name] = strategyValue(name, bar) }
        }
        for (name, value) in series.merging(working.variables, uniquingKeysWith: { $1 })
        where !name.hasPrefix(Self.internalPrefix) {
            working.histories[name, default: []].append(value)
        }
    }

    func budget() throws {
        working.instructions += 1
        if working.instructions > limits.instructionsPerBar {
            throw PineDiagnostic.error(
                "PINE8004", .resource, "Per-bar instruction limit exceeded.", .zero)
        }
    }

    func allocate() -> Int {
        working.nextReference += 1
        return working.nextReference
    }
}
