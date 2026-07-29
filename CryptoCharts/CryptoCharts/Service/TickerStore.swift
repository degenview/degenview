import Foundation

/// Persists user's ticker list to a JSON file in Application Support.
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

    /// Load saved tickers. Returns empty array if no file exists yet.
    func load() -> [String] {
        guard FileManager.default.fileExists(atPath: storageURL.path) else {
            return []
        }

        do {
            let data = try Data(contentsOf: storageURL)
            let tickers = try JSONDecoder().decode([String].self, from: data)
            return tickers
        } catch {
            print("[TickerStore] Failed to load tickers: \(error.localizedDescription)")
            return []
        }
    }

    /// Save tickers to disk atomically.
    func save(_ tickers: [String]) {
        do {
            let data = try JSONEncoder().encode(tickers)
            try data.write(to: storageURL, options: .atomic)
        } catch {
            print("[TickerStore] Failed to save tickers: \(error.localizedDescription)")
        }
    }
}
