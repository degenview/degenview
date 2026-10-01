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
    /// Fields of each script-defined `type`, by type name.
    let types: [String: [PineTypeField]]
    /// Members of each script-defined `enum`, by enum name.
    let enums: [String: [PineEnumMember]]
    /// Variable each top-level `input.*` call site initialises, so input overrides stay keyed
    /// by name even when titles change.
    let inputVariables: [Int: String]
    var committed = PineRuntimeState()
    var working = PineRuntimeState()
    /// `varip` values, which survive the rollback between realtime ticks of one bar.
    var intrabar: [String: PineRuntimeValue] = [:]
    /// Open time of the bar most recently executed, confirmed or not.
    var lastOpenTime: Date?
    /// Index and open time of the final bar, for `last_bar_index` and `last_bar_time`. Known up front
    /// for history; a realtime bar that opens past it becomes the new last bar.
    /// The chart script's variables while a `request.security` expression runs, so the expression can
    /// read inputs and constants without them entering its own histories. Nil otherwise.
    var securityGlobals: [String: PineRuntimeValue]?
    var lastBarIndex = -1
    var lastBarTime: Date?
    /// Open time of the bar most recently committed. Anything at or before it is stale.
    var lastCommittedOpenTime: Date?
    /// Call sites that already fired `alert.freq_once_per_bar` on the current bar. Like `intrabar` it
    /// escapes rollback, otherwise every realtime tick would fire the alert again.
    var oncePerBarLedger: Set<Int> = []
    /// Alerts raised by the latest `execute` on a realtime bar. Historical executions never populate it, so
    /// loading history cannot notify.
    private(set) var emittedAlerts: [PineAlertEvent] = []
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
        var types: [String: [PineTypeField]] = [:]
        var enums: [String: [PineEnumMember]] = [:]
        for statement in program.statements {
            switch statement {
            case .typeDeclaration(let name, let fields, _):
                types[name] = fields
            case .enumDeclaration(let name, let members, _):
                enums[name] = members
            case .function(let name, let parameters, let body, _):
                functions[name] = .init(parameters: parameters, body: body)
            case .declaration(let name, _, _, .call(_, _, let site, _), _):
                inputVariables[site] = name
            default: break
            }
        }
        self.functions = functions
        self.types = types
        self.enums = enums
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
        lastCommittedOpenTime = nil
        oncePerBarLedger = []
        emittedAlerts = []
        barSeconds = 0
        lastBarIndex = -1
        lastBarTime = nil
        mintick = suppliedMintick ?? Self.defaultMintick
    }

    // MARK: - Running

    /// Runs `bars` as one all-historical pass and returns the resulting output.
    func evaluate(bars: [KlineData]) throws -> PineRuntimeResult {
        let states = try load(history: bars)
        return .init(output: output(), diagnostics: [], barStates: states)
    }

    /// Resets the session and executes `bars` as confirmed history, committing each. Pass
    /// `precedesLiveBar` when a forming bar will follow through `execute`, so the last history bar is
    /// not reported as `barstate.islast`. `barSeconds` seeds the bar length when history is too short to
    /// learn it.
    @discardableResult
    func load(history bars: [KlineData], precedesLiveBar: Bool = false, barSeconds seed: Double? = nil)
        throws -> [PineBarFlags]
    {
        reset(inputs: inputs)
        if suppliedMintick == nil { mintick = Self.inferredMintick(bars) }
        if let seed, seed > 0 { barSeconds = seed }
        if bars.count > 1 {
            barSeconds = bars[bars.count - 1].openTime.timeIntervalSince(bars[bars.count - 2].openTime)
        }
        lastBarIndex = bars.count - 1 + (precedesLiveBar ? 1 : 0)
        lastBarTime = bars.last.map { $0.openTime.addingTimeInterval(precedesLiveBar ? barSeconds : 0) }
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
            let isLastHistory = index == bars.count - 1
            states.append(
                try execute(
                    .init(candle: bar, phase: .historical), isLast: isLastHistory && !precedesLiveBar,
                    isLastHistory: isLastHistory))
        }
        return states
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
    /// a realtime tick runs on a scratch copy of the state (rollback). Bar identity is the open time:
    /// an event for a bar older than the last one executed, or for one already committed, is rejected
    /// with `PineExecutionRejection` rather than advancing the series a second time.
    @discardableResult
    func execute(_ event: PineBarEvent, isLast: Bool = true, isLastHistory: Bool? = nil) throws -> PineBarFlags {
        let openTime = event.candle.openTime
        if let committedTime = lastCommittedOpenTime, openTime <= committedTime {
            throw openTime < committedTime ? PineExecutionRejection.staleBar : .duplicateBar
        }
        if let previous = lastOpenTime, openTime < previous { throw PineExecutionRejection.staleBar }
        let isNew = openTime != lastOpenTime
        if isNew {
            intrabar = [:]
            oncePerBarLedger = []
        }
        emittedAlerts = []
        if isNew, let previous = lastOpenTime {
            barSeconds = event.candle.openTime.timeIntervalSince(previous)
        }
        let confirmed = event.phase.isConfirmed
        let realtime = event.phase.isRealtime
        working = committed
        working.barIndex = committed.barIndex + 1
        if working.barIndex >= lastBarIndex {
            lastBarIndex = working.barIndex
            lastBarTime = openTime
        }
        working.instructions = 0
        if !isNew { for (key, value) in intrabar { working.variables[key] = value } }
        let flags = PineBarFlags(
            isFirst: working.barIndex == 0, isLast: isLast, isHistory: !realtime, isRealtime: realtime,
            isNew: isNew, isConfirmed: confirmed, isLastConfirmedHistory: !realtime && (isLastHistory ?? isLast))
        var context = PineRuntimeContext(bar: event.candle, flags: flags)
        if isStrategy {
            working.broker.process(bar: event.candle, barIndex: working.barIndex, mintick: mintick)
        }
        try run(program.statements, &context)
        if isStrategy { finishStrategyBar(event.candle) }
        if confirmed {
            commitHistories(event.candle, releasingCommitted: true)
            committed = working
            intrabar = [:]
            lastCommittedOpenTime = openTime
        }
        lastOpenTime = openTime
        return flags
    }

    private func finishStrategyBar(_ candle: KlineData) {
        if program.declaration.strategy?.processOrdersOnClose == true {
            working.broker.fillMarketOrdersAtClose(
                bar: candle, barIndex: working.barIndex, mintick: mintick)
        }
        working.broker.recordEquity(close: candle.closePrice)
    }

    /// Notes an alert this execution raised on a realtime bar. Called by the alert builtins.
    func emit(_ alert: PineAlertEvent) { emittedAlerts.append(alert) }

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
            // A linefill disappears with either of its lines.
            linefills: working.linefills.values.filter {
                working.lines[$0.line1] != nil && working.lines[$0.line2] != nil
            }.sorted { $0.id < $1.id },
            polylines: working.polylines.values.sorted { $0.id < $1.id },
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
    func commitHistories(_ bar: KlineData, releasingCommitted: Bool = false) {
        var series: [String: PineRuntimeValue] = [
            "open": .float(bar.openPrice), "high": .float(bar.highPrice), "low": .float(bar.lowPrice),
            "close": .float(bar.closePrice), "volume": .float(bar.volume),
            "hl2": .float((bar.highPrice + bar.lowPrice) / 2),
            "hlc3": .float((bar.highPrice + bar.lowPrice + bar.closePrice) / 3),
            "ohlc4": .float((bar.openPrice + bar.highPrice + bar.lowPrice + bar.closePrice) / 4),
            "time": .int(PineTime.milliseconds(bar.openTime)),
            "time_close": .int(PineTime.milliseconds(bar.openTime) + (Int(pine: barSeconds * 1000) ?? 0)),
            "bar_index": .int(working.barIndex),
        ]
        if isStrategy {
            for name in Self.strategySeries { series[name] = strategyValue(name, bar) }
        }
        // `working` and `committed` share every history array, and appending to a shared array copies it:
        // once per variable per bar, which made a long script quadratic. Letting go of the other holder
        // first lets the appends happen in place.
        var histories = working.histories
        working.histories = [:]
        if releasingCommitted { committed.histories = [:] }
        for (name, value) in series.merging(working.variables, uniquingKeysWith: { $1 })
        where !name.hasPrefix(Self.internalPrefix) {
            histories[name, default: []].append(value)
        }
        // A subscripted expression that was not reached this bar still takes its slot, so offsets stay aligned.
        let expressionKeys = Set(histories.keys.filter { $0.hasPrefix(Self.internalPrefix) })
            .union(working.expressionValues.keys)
        for key in expressionKeys {
            histories[key, default: []].append(working.expressionValues[key] ?? .na)
        }
        working.histories = histories
        working.expressionValues = [:]
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
