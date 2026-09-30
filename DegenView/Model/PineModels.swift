import Foundation

enum ScriptType: String, Codable, CaseIterable, Sendable, Identifiable {
    case indicator, strategy, library
    var id: String { rawValue }
    var displayName: String { rawValue.capitalized }
}

extension Notification.Name {
    static let localScriptsDidChange = Notification.Name("DegenView.localScriptsDidChange")
}

enum CompileStatus: String, Codable, CaseIterable, Sendable {
    case notCompiled, valid, warning, error
}

struct ScriptVersion: Codable, Equatable, Identifiable, Sendable {
    var id: UUID
    var scriptID: UUID
    var createdAt: Date
    var source: String
    var compileStatus: CompileStatus
}

struct ScriptDraft: Codable, Equatable, Sendable {
    var scriptID: UUID
    var source: String
    var modifiedAt: Date
    var basedOnRevisionID: UUID?
}

struct ScriptCompileRecord: Codable, Equatable, Sendable {
    var sourceHash: String
    var compilerVersion: String
    var pineVersion: Int?
    var status: CompileStatus
    var diagnostics: [PineDiagnostic]
    var declaration: PineDeclarationMetadata?
    var compiledAt: Date
}

struct LocalScript: Codable, Equatable, Identifiable, Sendable {
    var id: UUID
    var name: String
    var type: ScriptType
    var source: String
    var latestRevisionID: UUID?
    var createdAt: Date
    var modifiedAt: Date
    var lastOpenedAt: Date?
    var isFavorite: Bool
    var compileRecord: ScriptCompileRecord?
}

struct ChartScriptInstance: Codable, Equatable, Hashable, Identifiable, Sendable {
    enum UpdateStatus: String, Codable, Sendable { case current, available, missing }
    var id: UUID
    var scriptID: UUID
    var loadedRevisionID: UUID
    var inputs: [String: PineInputValue]
    var isVisible: Bool
    var styleOverrides: [String: String]
    var updateStatus: UpdateStatus

    init(
        id: UUID = UUID(), scriptID: UUID, loadedRevisionID: UUID,
        inputs: [String: PineInputValue] = [:], isVisible: Bool = true,
        styleOverrides: [String: String] = [:], updateStatus: UpdateStatus = .current
    ) {
        self.id = id
        self.scriptID = scriptID
        self.loadedRevisionID = loadedRevisionID
        self.inputs = inputs
        self.isVisible = isVisible
        self.styleOverrides = styleOverrides
        self.updateStatus = updateStatus
    }
}

struct PineSourcePosition: Codable, Equatable, Sendable {
    var line: Int
    var column: Int
    var offset: Int
}

struct PineSourceRange: Codable, Equatable, Sendable {
    var start: PineSourcePosition
    var end: PineSourcePosition
    static let zero = PineSourceRange(
        start: .init(line: 1, column: 1, offset: 0), end: .init(line: 1, column: 1, offset: 0))
}

enum PineDiagnosticSeverity: String, Codable, Sendable { case error, warning }
enum PineDiagnosticCategory: String, Codable, Sendable {
    case lexical, syntax, semantic, unsupported, resource, runtime, cancellation
}

struct PineDiagnostic: Error, Codable, Equatable, Sendable, Identifiable {
    var id: String { "\(code):\(range.start.offset):\(message)" }
    let code: String
    let severity: PineDiagnosticSeverity
    let category: PineDiagnosticCategory
    let message: String
    let range: PineSourceRange
}

enum PineValueType: String, Codable, Hashable, Sendable {
    case int, float, bool, string, color, plot, hline, void, unknown
    case line, label, box, table, array
    /// `input.time`: a millisecond timestamp carried as an `int`.
    case time
}
/// What a declaration wrote before its name: `[qualifier] [type] name = …`.
struct PineTypeAnnotation: Sendable, Equatable {
    var type: PineValueType?
    var qualifier: PineQualifier?

    init(type: PineValueType? = nil, qualifier: PineQualifier? = nil) {
        self.type = type
        self.qualifier = qualifier
    }
}
enum PineQualifier: Int, Codable, Comparable, Sendable {
    case constant, input, simple, series
    static func < (lhs: PineQualifier, rhs: PineQualifier) -> Bool { lhs.rawValue < rhs.rawValue }
}

enum PineInputValue: Codable, Equatable, Hashable, Sendable {
    case int(Int)
    case float(Double)
    case bool(Bool)
    case string(String)
    case color(UInt32)
    case source(String)
}

struct PineInputDefinition: Codable, Equatable, Hashable, Sendable, Identifiable {
    var id: String
    var type: PineValueType
    var defaultValue: PineInputValue
    var title: String?
    var tooltip: String?
    var group: String?
    var inline: String?
    var confirm: Bool
    var minValue: Double?
    var maxValue: Double?
    var step: Double?
    var options: [PineInputValue]?
}

struct PineInputSchema: Codable, Equatable, Sendable { var inputs: [PineInputDefinition] = [] }

struct PineDeclarationMetadata: Codable, Equatable, Sendable {
    var type: ScriptType = .indicator
    var pineVersion: Int? = nil
    var title: String
    var shortTitle: String?
    var overlay: Bool
    var format: String?
    var precision: Int?
    var maxBarsBack: Int?
    var maxLinesCount: Int? = nil
    var maxLabelsCount: Int? = nil
    var maxBoxesCount: Int? = nil
    /// Present for `strategy()` scripts.
    var strategy: PineStrategySettings? = nil
}

/// The `strategy()` declaration arguments the broker emulator honours.
struct PineStrategySettings: Codable, Equatable, Sendable {
    enum QuantityType: String, Codable, Sendable { case fixed, cash, percentOfEquity }
    enum CommissionType: String, Codable, Sendable { case percent, cashPerOrder, cashPerContract }

    var initialCapital = 1_000_000.0
    var quantityType = QuantityType.fixed
    var quantityValue = 1.0
    var commissionType = CommissionType.percent
    var commissionValue = 0.0
    /// Adverse slippage, in ticks, on market and stop fills.
    var slippage = 0
    var pyramiding = 1
    var currency: String?
    var processOrdersOnClose = false
}

/// Symbol facts a script can read through `syminfo.*`.
struct PineSymbolInfo: Equatable, Sendable {
    var ticker = ""
    var tickerID = ""
    var currency = "USD"
    var type = "crypto"
}

struct PineConfiguration: Codable, Equatable, Hashable, Sendable {
    var draftSource: String
    var appliedSource: String?
    var inputs: [String: PineInputValue]
    init(
        draftSource: String = "", appliedSource: String? = nil, inputs: [String: PineInputValue] = [:]
    ) {
        self.draftSource = draftSource
        self.appliedSource = appliedSource
        self.inputs = inputs
    }
}

enum PinePlotStyle: String, Codable, Sendable {
    case line, stepline, histogram, columns, area, circles, cross
}
enum PineMarkerKind: String, Codable, Sendable { case shape, character }

struct PinePlotOutput: Sendable, Identifiable {
    let id: Int
    var title: String?
    var values: [Double?]
    var color: UInt32
    var lineWidth: Int
    var style: PinePlotStyle
    /// Per-bar colors, parallel to `values`. `nil` falls back to `color`.
    var colors: [UInt32?] = []
    /// `display.*` bit mask; without `PineDisplay.pane` the plot is kept (a `fill()` may
    /// reference it) but neither drawn nor counted for the pane's value axis.
    var display = PineDisplay.all
    /// Baseline of `style_histogram` / `style_columns` / `style_area` plots.
    var histBase = 0.0
}

enum PineDisplay {
    static let none = 0
    static let pane = 1
    static let dataWindow = 2
    static let priceScale = 4
    static let statusLine = 8
    static let all = 15
}

struct PineHorizontalLine: Sendable, Identifiable {
    let id: Int
    var value: Double
    var color: UInt32
    var title: String?
}
struct PineMarkerOutput: Sendable, Identifiable {
    let id: Int
    var kind: PineMarkerKind
    var values: [Bool]
    var character: String?
    var color: UInt32
    var location: String
    var style: String
    /// Per-bar series value, parallel to `values`; used by `location.absolute`.
    var prices: [Double?] = []
    /// Per-bar colors, parallel to `values`.
    var colors: [UInt32?] = []
    var size: String = "size.auto"
    var display = PineDisplay.all
}
struct PineColorOutput: Sendable, Identifiable {
    let id: Int
    var colors: [UInt32?]
}

/// One bar of a gradient `fill()`: color runs from `topColor` at `top` to `bottomColor`
/// at `bottom`.
struct PineFillGradient: Sendable, Equatable {
    var top: Double
    var bottom: Double
    var topColor: UInt32
    var bottomColor: UInt32
}

/// Area between two plots; `colors` is per bar, parallel to the plots' values.
struct PineFillOutput: Sendable, Identifiable {
    let id: Int
    var plotA: Int
    var plotB: Int
    var colors: [UInt32?]
    /// Per-bar gradients for the `fill(p1, p2, top_value, bottom_value, top_color,
    /// bottom_color)` overload; parallel to `colors` and empty for flat fills.
    var gradients: [PineFillGradient?] = []
}

/// One bar of `plotcandle()`. A nil color hides that part.
struct PineCandleBar: Sendable, Equatable {
    var open: Double
    var high: Double
    var low: Double
    var close: Double
    var color: UInt32?
    var wickColor: UInt32?
    var borderColor: UInt32?
}

struct PineCandleOutput: Sendable, Identifiable {
    let id: Int
    var title: String?
    /// Per-bar candles, parallel to the script's bars; nil where the inputs were `na`.
    var bars: [PineCandleBar?]
    var display = PineDisplay.all
}

/// An `alert()` / `alertcondition()` that fired.
struct PineAlertEvent: Sendable, Identifiable, Equatable {
    let id: Int
    /// Call-site key, so `alert.freq_once_per_bar` can fire once per bar per call.
    var site: Int
    var bar: Int
    var time: Date
    var message: String
}

/// A round trip closed by the broker emulator. `profit` is net of both commissions.
struct PineTrade: Sendable, Identifiable, Equatable {
    let id: Int
    var entryID: String
    var exitID: String
    var isLong: Bool
    var quantity: Double
    var entryBar: Int
    var entryTime: Date
    var entryPrice: Double
    var exitBar: Int
    var exitTime: Date
    var exitPrice: Double
    var commission: Double
    var profit: Double
}

struct PineOpenTrade: Sendable, Identifiable, Equatable {
    var id: String { "\(entryID)@\(entryBar)" }
    var entryID: String
    var isLong: Bool
    var quantity: Double
    var entryBar: Int
    var entryTime: Date
    var entryPrice: Double
    var entryCommission: Double
}

/// Backtest result of a `strategy()` script.
struct PineStrategyReport: Sendable {
    var settings: PineStrategySettings
    var trades: [PineTrade]
    var openTrades: [PineOpenTrade]
    /// Mark-to-market equity after each bar, parallel to the script's bars.
    var equity: [Double]
    /// Unrealized P&L of the open position at the last bar, net of entry commissions.
    var openProfit: Double

    var netProfit: Double { trades.reduce(0) { $0 + $1.profit } }
    var grossProfit: Double { trades.filter { $0.profit > 0 }.reduce(0) { $0 + $1.profit } }
    var grossLoss: Double { -trades.filter { $0.profit < 0 }.reduce(0) { $0 + $1.profit } }
    var winCount: Int { trades.filter { $0.profit > 0 }.count }
    var winRate: Double? { trades.isEmpty ? nil : Double(winCount) / Double(trades.count) }
    var profitFactor: Double? { grossLoss > 0 ? grossProfit / grossLoss : nil }
    var averageTrade: Double? { trades.isEmpty ? nil : netProfit / Double(trades.count) }
    var finalEquity: Double { equity.last ?? settings.initialCapital }
    var returnFraction: Double {
        settings.initialCapital > 0 ? (finalEquity - settings.initialCapital) / settings.initialCapital : 0
    }

    /// Largest peak-to-trough fall of the equity curve, in currency and as a fraction of the peak.
    var maxDrawdown: (amount: Double, fraction: Double) {
        var peak = settings.initialCapital
        var worst = (amount: 0.0, fraction: 0.0)
        for value in equity {
            peak = max(peak, value)
            let drop = peak - value
            if drop > worst.amount { worst = (drop, peak > 0 ? drop / peak : 0) }
        }
        return worst
    }
}

/// Drawing objects anchor x coordinates to absolute `bar_index` values.
struct PineLineOutput: Sendable, Identifiable, Equatable {
    let id: Int
    var x1: Int
    var y1: Double
    var x2: Int
    var y2: Double
    var color: UInt32
    var width: Int
    var style: String
    var extend: String
}

struct PineLabelOutput: Sendable, Identifiable, Equatable {
    let id: Int
    var x: Int
    var y: Double
    var text: String
    var color: UInt32?
    var textColor: UInt32
    var style: String
    var size: String
}

struct PineBoxOutput: Sendable, Identifiable, Equatable {
    let id: Int
    var left: Int
    var top: Double
    var right: Int
    var bottom: Double
    var borderColor: UInt32?
    var borderWidth: Int
    var backgroundColor: UInt32?
}

struct PineTableCell: Sendable, Equatable {
    var column: Int
    var row: Int
    var text: String
    var textColor: UInt32
    var backgroundColor: UInt32?
    var textSize: String
}

struct PineTableOutput: Sendable, Identifiable, Equatable {
    let id: Int
    var position: String
    var columns: Int
    var rows: Int
    var backgroundColor: UInt32?
    var borderColor: UInt32?
    var borderWidth: Int
    var frameColor: UInt32?
    var frameWidth: Int
    var cells: [PineTableCell] = []
}

/// `chart.fg_color` / `chart.bg_color`, which follow the app's light or dark appearance.
struct PineChartTheme: Equatable, Hashable, Sendable {
    var foreground: UInt32
    var background: UInt32
    static let dark = PineChartTheme(foreground: 0xd1d4_dcff, background: 0x1317_22ff)
    static let light = PineChartTheme(foreground: 0x1317_22ff, background: 0xffff_ffff)
}

struct PineVisualOutput: Sendable {
    var overlay: Bool
    /// Number of bars the script ran over. Drawing objects use absolute `bar_index`
    /// coordinates; the renderer subtracts `barCount - visibleCandles` to map them.
    var barCount: Int = 0
    var plots: [PinePlotOutput] = []
    var hlines: [PineHorizontalLine] = []
    var markers: [PineMarkerOutput] = []
    var backgrounds: [PineColorOutput] = []
    var barColors: [PineColorOutput] = []
    var fills: [PineFillOutput] = []
    var lines: [PineLineOutput] = []
    var labels: [PineLabelOutput] = []
    var boxes: [PineBoxOutput] = []
    var tables: [PineTableOutput] = []
    var candles: [PineCandleOutput] = []
    var alerts: [PineAlertEvent] = []
    var strategy: PineStrategyReport?
    static let empty = PineVisualOutput(overlay: true)
}

enum PineBarPhase: Sendable {
    case historical
    case realtimeTick(isNew: Bool)
    case realtimeClose(isNew: Bool)
}
struct PineBarEvent: Sendable {
    var candle: KlineData
    var phase: PineBarPhase
}

struct PineLimits: Sendable {
    var sourceCharacters = 100_000, tokens = 50_000, astNodes = 50_000, irInstructions = 100_000
    var instructionsPerBar = 100_000, callDepth = 64, visualOutputs = 64, historyBars = 1_000_000
    var runtimeBytes = 256 * 1_024 * 1_024
    var deadline: TimeInterval = 10
    static let `default` = PineLimits()
}
