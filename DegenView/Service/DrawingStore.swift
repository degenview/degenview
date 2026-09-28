import Combine
import Foundation

/// App-wide trend-line persistence, keyed by the instrument rather than a chart card.
/// Every chart showing the same source+ticker observes the same entry.
@MainActor
final class DrawingStore: ObservableObject {
    static let shared = DrawingStore()

    @Published private(set) var linesByInstrument: [String: [TrendLine]]
    @Published private(set) var fibsByInstrument: [String: [FibonacciRetracementDrawing]]

    private let database: AppDatabase

    init(database: AppDatabase = .shared, legacyDirectory: URL = AppSupport.directory) {
        self.database = database
        database.importLegacyDrawings(
            TrendLine.self, filename: "drawings.json", kind: .trendLine, directory: legacyDirectory)
        database.importLegacyDrawings(
            FibonacciRetracementDrawing.self, filename: "fib-drawings.json", kind: .fibonacci,
            directory: legacyDirectory)
        linesByInstrument = database.drawings(TrendLine.self, kind: .trendLine)
        fibsByInstrument = database.drawings(FibonacciRetracementDrawing.self, kind: .fibonacci)
    }

    func fibs(ticker: String, source: DataSourceType) -> [FibonacciRetracementDrawing] {
        fibsByInstrument[key(ticker: ticker, source: source)] ?? []
    }

    func save(_ fibs: [FibonacciRetracementDrawing], ticker: String, source: DataSourceType) {
        let instrument = key(ticker: ticker, source: source)
        fibsByInstrument[instrument] = fibs
        database.saveDrawings(fibs, instrument: instrument, kind: .fibonacci)
    }

    func key(ticker: String, source: DataSourceType) -> String {
        "\(source.rawValue):\(ticker.trimmingCharacters(in: .whitespacesAndNewlines).lowercased())"
    }

    func lines(ticker: String, source: DataSourceType) -> [TrendLine] {
        linesByInstrument[key(ticker: ticker, source: source)] ?? []
    }

    func save(_ lines: [TrendLine], ticker: String, source: DataSourceType) {
        let instrument = key(ticker: ticker, source: source)
        // Keep an explicit empty entry. Besides representing deletion, it prevents an
        // old saved view containing legacy lines from importing them again later.
        linesByInstrument[instrument] = lines
        database.saveDrawings(lines, instrument: instrument, kind: .trendLine)
    }

    /// Moves lines persisted by older versions out of a tab/view config. An existing
    /// instrument entry wins, since it is already the shared source of truth.
    func importLegacy(_ lines: [TrendLine], ticker: String, source: DataSourceType) {
        let instrument = key(ticker: ticker, source: source)
        guard !lines.isEmpty, linesByInstrument[instrument] == nil else { return }
        save(lines, ticker: ticker, source: source)
    }

    func setLine(_ line: TrendLine?, at preferredIndex: Int, instrument: String, id: UUID) {
        var lines = linesByInstrument[instrument] ?? []
        lines.removeAll { $0.id == id }
        if let line { lines.insert(line, at: min(max(0, preferredIndex), lines.count)) }
        linesByInstrument[instrument] = lines
        database.saveDrawings(lines, instrument: instrument, kind: .trendLine)
    }

    func setFibonacci(
        _ drawing: FibonacciRetracementDrawing?, at preferredIndex: Int, instrument: String, id: UUID
    ) {
        var drawings = fibsByInstrument[instrument] ?? []
        drawings.removeAll { $0.id == id }
        if let drawing { drawings.insert(drawing, at: min(max(0, preferredIndex), drawings.count)) }
        fibsByInstrument[instrument] = drawings
        database.saveDrawings(drawings, instrument: instrument, kind: .fibonacci)
    }
}
