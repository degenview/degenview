import Foundation

/// Persists user's ticker list to a JSON file in Application Support.
/// Stores `TickerConfig` (symbol + source). Falls back to legacy `[String]` format.
final class TickerStore {
    private let storageURL: URL

    init() {
        let appSupport = FileManager.default.urls(
            for: .applicationSupportDirectory,
            in: .userDomainMask
        ).first!

        let directory = appSupport.appendingPathComponent("CryptoCharts", isDirectory: true)

        // Create directory if it doesn't exist
        try? FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)

        self.storageURL = directory.appendingPathComponent("tickers.json")
    }

    /// Load saved tickers. Migrates legacy `[String]` → `[TickerConfig]` with `.binance` source.
    func load() -> [TickerConfig] {
        guard FileManager.default.fileExists(atPath: storageURL.path) else {
            return []
        }

        guard let data = try? Data(contentsOf: storageURL) else { return [] }

        // Try current format first
        if let configs = try? JSONDecoder().decode([TickerConfig].self, from: data) {
            return configs
        }

        // Fall back to legacy [String] format — migrate to .binance
        if let strings = try? JSONDecoder().decode([String].self, from: data) {
            print("[TickerStore] Migrated \(strings.count) legacy tickers to .binance")
            return strings.map { TickerConfig(symbol: $0, source: .binance) }
        }

        print("[TickerStore] Failed to decode tickers")
        return []
    }

    /// Save ticker configs to disk atomically.
    func save(_ configs: [TickerConfig]) {
        do {
            let data = try JSONEncoder().encode(configs)
            try data.write(to: storageURL, options: .atomic)
        } catch {
            print("[TickerStore] Failed to save tickers: \(error.localizedDescription)")
        }
    }
}
