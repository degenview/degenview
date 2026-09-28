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

    init(database: AppDatabase = .shared) {
        self.database = database
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
        linesByInstrument[instrument] = lines
        database.saveDrawings(lines, instrument: instrument, kind: .trendLine)
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
