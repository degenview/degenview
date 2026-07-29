import Foundation

/// Persists saved chart views to a JSON file in Application Support.
final class ViewStore {
    private let storageURL: URL

    init() {
        let appSupport = FileManager.default.urls(
            for: .applicationSupportDirectory,
            in: .userDomainMask
        ).first!

        let directory = appSupport.appendingPathComponent("CryptoCharts", isDirectory: true)
        try? FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)

        self.storageURL = directory.appendingPathComponent("views.json")
    }

    func load() -> [SavedView] {
        guard FileManager.default.fileExists(atPath: storageURL.path) else {
            return []
        }

        do {
            let data = try Data(contentsOf: storageURL)
            return try JSONDecoder().decode([SavedView].self, from: data)
        } catch {
            print("[ViewStore] Failed to load views: \(error.localizedDescription)")
            return []
        }
    }

    func save(_ views: [SavedView]) {
        do {
            let data = try JSONEncoder().encode(views)
            try data.write(to: storageURL, options: .atomic)
        } catch {
            print("[ViewStore] Failed to save views: \(error.localizedDescription)")
        }
    }
}
