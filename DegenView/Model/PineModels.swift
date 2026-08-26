import Foundation

enum ScriptType: String, Codable, CaseIterable, Sendable, Identifiable {
    case indicator, strategy, library
    var id: String { rawValue }
    var displayName: String { rawValue.capitalized }
}

/// A distinct scene value prevents script editor requests from being routed to the
/// chart `WindowGroup`, which is also keyed by raw UUID values.
struct ScriptEditorWindowID: Codable, Hashable, Sendable {
    /// Unique even for unsaved editors, so several new scripts can be composed at once.
    var windowID: UUID
    var scriptID: UUID?

    init(scriptID: UUID?) {
        self.windowID = scriptID ?? UUID()
        self.scriptID = scriptID
    }
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
}
struct PineColorOutput: Sendable, Identifiable {
    let id: Int
    var colors: [UInt32?]
}

struct PineVisualOutput: Sendable {
    var overlay: Bool
    var plots: [PinePlotOutput] = []
    var hlines: [PineHorizontalLine] = []
    var markers: [PineMarkerOutput] = []
    var backgrounds: [PineColorOutput] = []
    var barColors: [PineColorOutput] = []
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
