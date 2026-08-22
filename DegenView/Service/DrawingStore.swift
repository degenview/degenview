import Foundation
import Combine

/// App-wide trend-line persistence, keyed by the instrument rather than a chart card.
/// Every chart showing the same source+ticker observes the same entry.
@MainActor
final class DrawingStore: ObservableObject {
    static let shared = DrawingStore()

    @Published private(set) var linesByInstrument: [String: [TrendLine]]

    private let store = JSONStore<[String: [TrendLine]]>(filename: "drawings.json")

    private init() {
        linesByInstrument = store.load() ?? [:]
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
        store.save(linesByInstrument)
    }

    /// Moves lines persisted by older versions out of a tab/view config. An existing
    /// instrument entry wins, since it is already the shared source of truth.
    func importLegacy(_ lines: [TrendLine], ticker: String, source: DataSourceType) {
        let instrument = key(ticker: ticker, source: source)
        guard !lines.isEmpty, linesByInstrument[instrument] == nil else { return }
        save(lines, ticker: ticker, source: source)
    }
}
